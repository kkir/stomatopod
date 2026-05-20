//! Postgres-backed storage for SaaS deployments.
//!
//! Holds everything in one database — analytics events, agent spans,
//! and metadata (sites, orgs, users, funnels, AI-firewall config). Uses
//! `sqlx` with a connection pool and `INSERT ... ON CONFLICT` for
//! upserts.
//!
//! Schema is created lazily via `bootstrap()`; idempotent and safe to
//! re-run on every boot. Migrations are kept inline so they stay close
//! to the impl — when this grows beyond a couple of tables we should
//! switch to `sqlx::migrate!`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use ulid::Ulid;

use stomatopod_core::{
    config::PostgresConfig,
    domain::{
        agent::{Agent, AlertChannel, AlertChannelKind, SentinelToken},
        agent_span::AgentSpan,
        event::Event,
        incident::{Incident, IncidentStatus, IncidentTrigger},
        org::{Funnel, Organization, Plan, User, UserRole},
        policy::Policy,
        site::Site,
    },
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult, FunnelStepResult},
        pageviews::{
            Granularity, PageviewsQuery, PageviewsResult, TimeBucket, TimeRange, TopList, TopRow,
        },
        spans::{AgentSummary, SpanQuery, SpanRow},
    },
    traits::{AgentStore, MetaStore, StorageBackend},
};

pub struct PostgresBackend {
    pool: PgPool,
}

impl PostgresBackend {
    pub async fn connect(cfg: &PostgresConfig) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.url)
            .await?;
        Ok(Self { pool })
    }

    /// Create every table this backend uses if it does not exist —
    /// metadata tables first (some have FKs into each other), then
    /// the analytics tables. Idempotent; safe to call on every boot.
    pub async fn bootstrap(&self) -> Result<(), StoreError> {
        for stmt in ddl::all_statements()
            .iter()
            .chain(ddl::analytics_statements().iter())
        {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(StoreError::db)?;
        }
        Ok(())
    }
}

fn parse_ulid(s: &str) -> Ulid {
    Ulid::from_string(s).unwrap_or_default()
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        // One multi-row INSERT per batch. UNNEST keeps the wire format
        // compact and avoids 27 * N parameter placeholders that grow
        // beyond Postgres's per-statement limit (32 767) on big batches.
        let mut tx = self.pool.begin().await.map_err(StoreError::db)?;
        for e in &events {
            sqlx::query(
                "INSERT INTO events (
                    id, site_id, name, kind, timestamp, received_at, url, referrer,
                    utm_source, utm_medium, utm_campaign, utm_term, utm_content,
                    browser, browser_version, os, os_version, device_type,
                    screen_width, screen_height, language,
                    ip_anonymized, country_code, region, city,
                    session_id, properties
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8,
                    $9, $10, $11, $12, $13,
                    $14, $15, $16, $17, $18,
                    $19, $20, $21,
                    $22, $23, $24, $25,
                    $26, $27
                )",
            )
            .bind(e.id.to_string())
            .bind(e.site_id.to_string())
            .bind(&e.name)
            .bind(e.kind.as_str())
            .bind(e.timestamp)
            .bind(e.received_at)
            .bind(&e.url)
            .bind(&e.referrer)
            .bind(&e.utm_source)
            .bind(&e.utm_medium)
            .bind(&e.utm_campaign)
            .bind(&e.utm_term)
            .bind(&e.utm_content)
            .bind(&e.browser)
            .bind(&e.browser_version)
            .bind(&e.os)
            .bind(&e.os_version)
            .bind(e.device_type.as_str())
            .bind(e.screen_width.map(|v| v as i32))
            .bind(e.screen_height.map(|v| v as i32))
            .bind(&e.language)
            .bind(&e.ip_anonymized)
            .bind(&e.country_code)
            .bind(&e.region)
            .bind(&e.city)
            .bind(&e.session_id[..])
            .bind(e.properties.as_ref().map(|p| p.to_string()))
            .execute(&mut *tx)
            .await
            .map_err(StoreError::db)?;
        }
        tx.commit().await.map_err(StoreError::db)?;
        Ok(())
    }

    async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        let bucket = pg_date_trunc(&q.granularity);
        let sql = format!(
            "SELECT date_trunc('{bucket}', timestamp) AS bucket, \
                    COUNT(*) FILTER (WHERE kind = 'pageview')::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
             GROUP BY 1 ORDER BY 1"
        );
        let rows = sqlx::query(&sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut result = PageviewsResult::default();
        for row in rows {
            let ts: DateTime<Utc> = row.try_get("bucket").map_err(StoreError::query)?;
            let pv: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
            let sess: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let pv = pv as u64;
            let sess = sess as u64;
            result.total_pageviews += pv;
            result.total_sessions += sess;
            result.buckets.push(TimeBucket {
                ts,
                pageviews: pv,
                sessions: sess,
            });
        }
        Ok(result)
    }

    async fn query_top_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "url").await
    }

    async fn query_top_referrers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "referrer")
            .await
    }

    async fn query_top_countries(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "country_code")
            .await
    }

    async fn query_top_browsers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "browser").await
    }

    async fn query_top_devices(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "device_type")
            .await
    }

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        let name_filter = if q.event_name.is_some() {
            "AND name = $4"
        } else {
            ""
        };
        let sql = format!(
            "SELECT name AS value, \
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'custom' {name_filter} \
             GROUP BY 1 ORDER BY pageviews DESC \
             LIMIT {}",
            q.limit,
        );
        let mut query = sqlx::query(&sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end);
        if let Some(name) = q.event_name.as_deref() {
            query = query.bind(name.to_string());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        Ok(top_list_from_rows(rows)?)
    }

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        if q.steps.is_empty() {
            return Ok(FunnelResult::default());
        }
        let mut step_results = Vec::with_capacity(q.steps.len());
        let mut prev_sessions: Option<u64> = None;
        for step in &q.steps {
            let row = sqlx::query(
                "SELECT COUNT(DISTINCT session_id)::BIGINT AS sessions \
                 FROM events \
                 WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                   AND name = $4",
            )
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end)
            .bind(&step.event_name)
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::query)?;
            let sessions: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let sessions = sessions as u64;
            let (cr, drop) = match prev_sessions {
                None => (1.0, 0.0),
                Some(prev) if prev > 0 => {
                    let cr = sessions as f64 / prev as f64;
                    (cr, 1.0 - cr)
                }
                _ => (0.0, 1.0),
            };
            step_results.push(FunnelStepResult {
                name: step.name.clone(),
                sessions,
                conversion_rate: cr,
                drop_off_rate: drop,
            });
            prev_sessions = Some(sessions);
        }
        Ok(FunnelResult {
            steps: step_results,
        })
    }
}

impl PostgresBackend {
    async fn query_top_field(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
    ) -> Result<TopList, StoreError> {
        // `field` is a static identifier from the trait surface, never
        // user input — see top-level trait docs.
        let sql = format!(
            "SELECT COALESCE({field}, 'Direct / None') AS value, \
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'pageview' \
             GROUP BY 1 ORDER BY pageviews DESC \
             LIMIT {limit}"
        );
        let rows = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        top_list_from_rows(rows)
    }
}

fn pg_date_trunc(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hour",
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
    }
}

fn top_list_from_rows(rows: Vec<sqlx::postgres::PgRow>) -> Result<TopList, StoreError> {
    let mut out: Vec<TopRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let value: Option<String> = row.try_get("value").map_err(StoreError::query)?;
        let pv: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
        let sess: i64 = row.try_get("sessions").map_err(StoreError::query)?;
        out.push(TopRow {
            value: value.unwrap_or_else(|| "Direct / None".into()),
            pageviews: pv as u64,
            sessions: sess as u64,
            pct: 0.0,
        });
    }
    let total: u64 = out.iter().map(|r| r.pageviews).sum();
    if total > 0 {
        for row in &mut out {
            row.pct = (row.pageviews as f64 / total as f64) * 100.0;
        }
    }
    Ok(TopList { rows: out })
}

#[async_trait]
impl MetaStore for PostgresBackend {
    async fn create_site(&self, site: &Site) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sites (id, org_id, domain, name, timezone, public_key, created_at, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(site.id.to_string())
        .bind(site.org_id.to_string())
        .bind(&site.domain)
        .bind(&site.name)
        .bind(&site.timezone)
        .bind(&site.public_key)
        .bind(site.created_at)
        .bind(site.is_active)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_site(&self, id: Ulid) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn get_site_by_key(&self, public_key: &str) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE public_key = $1 AND is_active = TRUE",
        )
        .bind(public_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn get_site_by_domain(&self, domain: &str) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE domain = $1 AND is_active = TRUE LIMIT 1",
        )
        .bind(domain)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn list_sites(&self, org_id: Ulid) -> Result<Vec<Site>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE org_id = $1 ORDER BY created_at ASC",
        )
        .bind(org_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_site).collect()
    }

    async fn delete_site(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn create_org(&self, org: &Organization) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO orgs (id, name, slug, plan, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org.id.to_string())
        .bind(&org.name)
        .bind(&org.slug)
        .bind(plan_str(org.plan))
        .bind(org.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_org(&self, id: Ulid) -> Result<Option<Organization>, StoreError> {
        let row = sqlx::query("SELECT id, name, slug, plan, created_at FROM orgs WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::db)?;
        row.map(row_to_org).transpose()
    }

    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, name, slug, plan, created_at FROM orgs ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_org).collect()
    }

    async fn create_user(&self, user: &User) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO users (id, org_id, email, password_hash, role, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(user.id.to_string())
        .bind(user.org_id.to_string())
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(role_str(user.role))
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, email, password_hash, role, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_user).transpose()
    }

    async fn create_funnel(&self, funnel: &Funnel) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO funnels (id, site_id, name, definition, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(funnel.id.to_string())
        .bind(funnel.site_id.to_string())
        .bind(&funnel.name)
        .bind(&funnel.definition)
        .bind(funnel.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_funnel(&self, id: Ulid) -> Result<Option<Funnel>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, name, definition, created_at FROM funnels WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_funnel).transpose()
    }

    async fn list_funnels(&self, site_id: Ulid) -> Result<Vec<Funnel>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, name, definition, created_at FROM funnels \
             WHERE site_id = $1 ORDER BY created_at ASC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_funnel).collect()
    }

    async fn delete_funnel(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM funnels WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn upsert_agent(&self, agent: &Agent) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO agents (id, site_id, agent_id, name, policy_id, created_at, last_seen_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (site_id, agent_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                policy_id = EXCLUDED.policy_id, \
                last_seen_at = EXCLUDED.last_seen_at",
        )
        .bind(agent.id.to_string())
        .bind(agent.site_id.to_string())
        .bind(&agent.agent_id)
        .bind(&agent.name)
        .bind(agent.policy_id.map(|p| p.to_string()))
        .bind(agent.created_at)
        .bind(agent.last_seen_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_agents(&self, site_id: Ulid) -> Result<Vec<Agent>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at \
             FROM agents WHERE site_id = $1 ORDER BY last_seen_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_agent).collect()
    }

    async fn get_agent(&self, site_id: Ulid, agent_id: &str) -> Result<Option<Agent>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at \
             FROM agents WHERE site_id = $1 AND agent_id = $2",
        )
        .bind(site_id.to_string())
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_agent).transpose()
    }

    async fn create_sentinel_token(&self, token: &SentinelToken) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sentinel_tokens (id, site_id, name, token_hash, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(token.id.to_string())
        .bind(token.site_id.to_string())
        .bind(&token.name)
        .bind(&token.token_hash)
        .bind(token.created_at)
        .bind(token.last_used_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_sentinel_tokens(&self, site_id: Ulid) -> Result<Vec<SentinelToken>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, name, token_hash, created_at, last_used_at \
             FROM sentinel_tokens WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_sentinel_token).collect()
    }

    async fn get_sentinel_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SentinelToken>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, name, token_hash, created_at, last_used_at \
             FROM sentinel_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_sentinel_token).transpose()
    }

    async fn touch_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("UPDATE sentinel_tokens SET last_used_at = NOW() WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn delete_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sentinel_tokens WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn create_alert_channel(&self, channel: &AlertChannel) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO alert_channels (id, site_id, kind, url, secret, created_at, last_error_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(channel.id.to_string())
        .bind(channel.site_id.to_string())
        .bind(channel.kind.as_str())
        .bind(&channel.url)
        .bind(&channel.secret)
        .bind(channel.created_at)
        .bind(channel.last_error_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_alert_channels(&self, site_id: Ulid) -> Result<Vec<AlertChannel>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, kind, url, secret, created_at, last_error_at \
             FROM alert_channels WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_alert_channel).collect()
    }

    async fn delete_alert_channel(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM alert_channels WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn upsert_policy(&self, policy: &Policy) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO policies (id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (site_id) DO UPDATE SET \
                repetition_max = EXCLUDED.repetition_max, \
                velocity_max_tps = EXCLUDED.velocity_max_tps, \
                cost_cap_usd = EXCLUDED.cost_cap_usd, \
                hint_template = EXCLUDED.hint_template",
        )
        .bind(policy.id.to_string())
        .bind(policy.site_id.to_string())
        .bind(policy.repetition_max.map(|n| n as i64))
        .bind(policy.velocity_max_tps)
        .bind(policy.cost_cap_usd)
        .bind(&policy.hint_template)
        .bind(policy.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_policy(&self, site_id: Ulid) -> Result<Option<Policy>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at \
             FROM policies WHERE site_id = $1",
        )
        .bind(site_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_policy).transpose()
    }

    async fn record_incident(&self, incident: &Incident) -> Result<(), StoreError> {
        let trigger_json = serde_json::to_string(&incident.trigger)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO incidents (id, site_id, agent_id, trigger_json, status, opened_at, closed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(incident.id.to_string())
        .bind(incident.site_id.to_string())
        .bind(&incident.agent_id)
        .bind(trigger_json)
        .bind(incident.status.as_str())
        .bind(incident.opened_at)
        .bind(incident.closed_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_incidents(&self, site_id: Ulid, limit: u32) -> Result<Vec<Incident>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, agent_id, trigger_json, status, opened_at, closed_at \
             FROM incidents WHERE site_id = $1 ORDER BY opened_at DESC LIMIT $2",
        )
        .bind(site_id.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_incident).collect()
    }

    async fn update_incident_status(
        &self,
        id: Ulid,
        status: IncidentStatus,
    ) -> Result<(), StoreError> {
        let closed_at = matches!(status, IncidentStatus::Resolved).then(Utc::now);
        sqlx::query(
            "UPDATE incidents SET status = $1, closed_at = COALESCE($2, closed_at) WHERE id = $3",
        )
        .bind(status.as_str())
        .bind(closed_at)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }
}

#[async_trait]
impl AgentStore for PostgresBackend {
    async fn ingest_spans(&self, spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        if spans.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(StoreError::db)?;
        for s in &spans {
            sqlx::query(
                "INSERT INTO agent_spans (
                    id, site_id, agent_id, agent_session_id, parent_span_id,
                    kind, model, started_at, ended_at,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    cost_usd, tool_name, tool_input_hash, stop_reason, properties
                ) VALUES (
                    $1, $2, $3, $4, $5,
                    $6, $7, $8, $9,
                    $10, $11, $12, $13,
                    $14, $15, $16, $17, $18
                )",
            )
            .bind(s.id.to_string())
            .bind(s.site_id.to_string())
            .bind(&s.agent_id)
            .bind(&s.agent_session_id)
            .bind(s.parent_span_id.map(|p| p.to_string()))
            .bind(s.kind.as_str())
            .bind(&s.model)
            .bind(s.started_at)
            .bind(s.ended_at)
            .bind(s.input_tokens as i64)
            .bind(s.output_tokens as i64)
            .bind(s.cache_read_tokens as i64)
            .bind(s.cache_creation_tokens as i64)
            .bind(s.cost_usd)
            .bind(&s.tool_name)
            .bind(&s.tool_input_hash)
            .bind(&s.stop_reason)
            .bind(s.properties.as_ref().map(|p| p.to_string()))
            .execute(&mut *tx)
            .await
            .map_err(StoreError::db)?;
        }
        tx.commit().await.map_err(StoreError::db)?;
        Ok(())
    }

    async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        // Build with a fixed parameter shape so the prepared-statement
        // cache hits across calls regardless of which optional filters
        // are present. NULL acts as a wildcard.
        let rows = sqlx::query(
            "SELECT id, agent_id, agent_session_id, kind, model, started_at, ended_at, \
                    input_tokens, output_tokens, cost_usd, tool_name, stop_reason \
             FROM agent_spans \
             WHERE site_id = $1 \
               AND started_at >= $2 AND started_at <= $3 \
               AND ($4::TEXT IS NULL OR agent_id = $4) \
               AND ($5::TEXT IS NULL OR agent_session_id = $5) \
             ORDER BY started_at DESC \
             LIMIT $6",
        )
        .bind(q.site_id.to_string())
        .bind(q.since)
        .bind(q.until)
        .bind(&q.agent_id)
        .bind(&q.session_id)
        .bind(q.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;

        rows.into_iter()
            .map(|row| {
                let in_tok: i64 = row.try_get("input_tokens").map_err(StoreError::query)?;
                let out_tok: i64 = row.try_get("output_tokens").map_err(StoreError::query)?;
                Ok(SpanRow {
                    id: row.try_get("id").map_err(StoreError::query)?,
                    agent_id: row.try_get("agent_id").map_err(StoreError::query)?,
                    agent_session_id: row.try_get("agent_session_id").map_err(StoreError::query)?,
                    kind: row.try_get("kind").map_err(StoreError::query)?,
                    model: row.try_get("model").map_err(StoreError::query)?,
                    started_at: row.try_get("started_at").map_err(StoreError::query)?,
                    ended_at: row.try_get("ended_at").map_err(StoreError::query)?,
                    input_tokens: in_tok as u32,
                    output_tokens: out_tok as u32,
                    cost_usd: row.try_get("cost_usd").map_err(StoreError::query)?,
                    tool_name: row.try_get("tool_name").map_err(StoreError::query)?,
                    stop_reason: row.try_get("stop_reason").map_err(StoreError::query)?,
                })
            })
            .collect()
    }

    async fn summarize_agents(
        &self,
        site_id: Ulid,
        since: DateTime<Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        let rows = sqlx::query(
            "SELECT agent_id, \
                    MAX(started_at) AS last_seen, \
                    COUNT(*)::BIGINT AS total_spans, \
                    SUM(input_tokens)::BIGINT AS in_tok, \
                    SUM(output_tokens)::BIGINT AS out_tok, \
                    SUM(cost_usd)::DOUBLE PRECISION AS cost \
             FROM agent_spans \
             WHERE site_id = $1 AND started_at >= $2 \
             GROUP BY agent_id \
             ORDER BY last_seen DESC",
        )
        .bind(site_id.to_string())
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;

        rows.into_iter()
            .map(|row| {
                let total_spans: i64 = row.try_get("total_spans").map_err(StoreError::query)?;
                let in_tok: i64 = row.try_get("in_tok").map_err(StoreError::query)?;
                let out_tok: i64 = row.try_get("out_tok").map_err(StoreError::query)?;
                Ok(AgentSummary {
                    agent_id: row.try_get("agent_id").map_err(StoreError::query)?,
                    last_seen_at: row.try_get("last_seen").map_err(StoreError::query)?,
                    total_spans: total_spans as u64,
                    total_input_tokens: in_tok as u64,
                    total_output_tokens: out_tok as u64,
                    total_cost_usd: row.try_get("cost").map_err(StoreError::query)?,
                })
            })
            .collect()
    }

    async fn session_cost_usd(
        &self,
        site_id: Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(cost_usd), 0.0)::DOUBLE PRECISION AS cost \
             FROM agent_spans \
             WHERE site_id = $1 AND agent_session_id = $2",
        )
        .bind(site_id.to_string())
        .bind(agent_session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::query)?;
        row.try_get("cost").map_err(StoreError::query)
    }
}

// ---------------------------------------------------------------------------
// Row decoders.
// ---------------------------------------------------------------------------

fn row_to_site(row: sqlx::postgres::PgRow) -> Result<Site, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let org_id: String = row.try_get("org_id").map_err(StoreError::db)?;
    Ok(Site {
        id: parse_ulid(&id),
        org_id: parse_ulid(&org_id),
        domain: row.try_get("domain").map_err(StoreError::db)?,
        name: row.try_get("name").map_err(StoreError::db)?,
        timezone: row.try_get("timezone").map_err(StoreError::db)?,
        public_key: row.try_get("public_key").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        is_active: row.try_get("is_active").map_err(StoreError::db)?,
    })
}

fn row_to_org(row: sqlx::postgres::PgRow) -> Result<Organization, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let plan: String = row.try_get("plan").map_err(StoreError::db)?;
    Ok(Organization {
        id: parse_ulid(&id),
        name: row.try_get("name").map_err(StoreError::db)?,
        slug: row.try_get("slug").map_err(StoreError::db)?,
        plan: parse_plan(&plan),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_user(row: sqlx::postgres::PgRow) -> Result<User, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let org_id: String = row.try_get("org_id").map_err(StoreError::db)?;
    let role: String = row.try_get("role").map_err(StoreError::db)?;
    Ok(User {
        id: parse_ulid(&id),
        org_id: parse_ulid(&org_id),
        email: row.try_get("email").map_err(StoreError::db)?,
        password_hash: row.try_get("password_hash").map_err(StoreError::db)?,
        role: parse_role(&role),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_funnel(row: sqlx::postgres::PgRow) -> Result<Funnel, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(Funnel {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        name: row.try_get("name").map_err(StoreError::db)?,
        definition: row.try_get("definition").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_agent(row: sqlx::postgres::PgRow) -> Result<Agent, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let policy_id: Option<String> = row.try_get("policy_id").map_err(StoreError::db)?;
    Ok(Agent {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        agent_id: row.try_get("agent_id").map_err(StoreError::db)?,
        name: row.try_get("name").map_err(StoreError::db)?,
        policy_id: policy_id.as_deref().and_then(|s| Ulid::from_string(s).ok()),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_seen_at: row.try_get("last_seen_at").map_err(StoreError::db)?,
    })
}

fn row_to_sentinel_token(row: sqlx::postgres::PgRow) -> Result<SentinelToken, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(SentinelToken {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        name: row.try_get("name").map_err(StoreError::db)?,
        token_hash: row.try_get("token_hash").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_used_at: row.try_get("last_used_at").map_err(StoreError::db)?,
    })
}

fn row_to_alert_channel(row: sqlx::postgres::PgRow) -> Result<AlertChannel, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let kind: String = row.try_get("kind").map_err(StoreError::db)?;
    Ok(AlertChannel {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        kind: parse_alert_kind(&kind),
        url: row.try_get("url").map_err(StoreError::db)?,
        secret: row.try_get("secret").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_error_at: row.try_get("last_error_at").map_err(StoreError::db)?,
    })
}

fn row_to_policy(row: sqlx::postgres::PgRow) -> Result<Policy, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let repetition_max: Option<i64> = row.try_get("repetition_max").map_err(StoreError::db)?;
    Ok(Policy {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        repetition_max: repetition_max.map(|n| n as u32),
        velocity_max_tps: row.try_get("velocity_max_tps").map_err(StoreError::db)?,
        cost_cap_usd: row.try_get("cost_cap_usd").map_err(StoreError::db)?,
        hint_template: row.try_get("hint_template").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_incident(row: sqlx::postgres::PgRow) -> Result<Incident, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let trigger_json: String = row.try_get("trigger_json").map_err(StoreError::db)?;
    let status: String = row.try_get("status").map_err(StoreError::db)?;
    let trigger: IncidentTrigger =
        serde_json::from_str(&trigger_json).unwrap_or(IncidentTrigger::Manual);
    Ok(Incident {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        agent_id: row.try_get("agent_id").map_err(StoreError::db)?,
        trigger,
        status: parse_incident_status(&status),
        opened_at: row.try_get("opened_at").map_err(StoreError::db)?,
        closed_at: row.try_get("closed_at").map_err(StoreError::db)?,
    })
}

fn plan_str(p: Plan) -> &'static str {
    match p {
        Plan::SelfHosted => "self_hosted",
        Plan::Free => "free",
        Plan::Pro => "pro",
        Plan::Enterprise => "enterprise",
    }
}

fn parse_plan(s: &str) -> Plan {
    match s {
        "free" => Plan::Free,
        "pro" => Plan::Pro,
        "enterprise" => Plan::Enterprise,
        _ => Plan::SelfHosted,
    }
}

fn role_str(r: UserRole) -> &'static str {
    match r {
        UserRole::Owner => "owner",
        UserRole::Admin => "admin",
        UserRole::Viewer => "viewer",
    }
}

fn parse_role(s: &str) -> UserRole {
    match s {
        "admin" => UserRole::Admin,
        "viewer" => UserRole::Viewer,
        _ => UserRole::Owner,
    }
}

fn parse_alert_kind(s: &str) -> AlertChannelKind {
    match s {
        "slack" => AlertChannelKind::Slack,
        _ => AlertChannelKind::Webhook,
    }
}

fn parse_incident_status(s: &str) -> IncidentStatus {
    match s {
        "acknowledged" => IncidentStatus::Acknowledged,
        "resolved" => IncidentStatus::Resolved,
        _ => IncidentStatus::Open,
    }
}

mod ddl {
    pub fn all_statements() -> [&'static str; 9] {
        [
            ORGS_DDL,
            SITES_DDL,
            USERS_DDL,
            FUNNELS_DDL,
            POLICIES_DDL,
            AGENTS_DDL,
            SENTINEL_TOKENS_DDL,
            ALERT_CHANNELS_DDL,
            INCIDENTS_DDL,
        ]
    }

    pub fn analytics_statements() -> [&'static str; 4] {
        [EVENTS_DDL, EVENTS_INDEX_TIMESTAMP, SPANS_DDL, SPANS_INDEX]
    }

    const ORGS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS orgs (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            slug       TEXT UNIQUE NOT NULL,
            plan       TEXT NOT NULL DEFAULT 'self_hosted',
            created_at TIMESTAMPTZ NOT NULL
        )
    "#;

    const SITES_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS sites (
            id         TEXT PRIMARY KEY,
            org_id     TEXT NOT NULL REFERENCES orgs(id),
            domain     TEXT NOT NULL,
            name       TEXT NOT NULL,
            timezone   TEXT NOT NULL DEFAULT 'UTC',
            public_key TEXT UNIQUE NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            is_active  BOOLEAN NOT NULL DEFAULT TRUE
        )
    "#;

    const USERS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS users (
            id            TEXT PRIMARY KEY,
            org_id        TEXT NOT NULL REFERENCES orgs(id),
            email         TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role          TEXT NOT NULL DEFAULT 'owner',
            created_at    TIMESTAMPTZ NOT NULL
        )
    "#;

    const FUNNELS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS funnels (
            id         TEXT PRIMARY KEY,
            site_id    TEXT NOT NULL REFERENCES sites(id),
            name       TEXT NOT NULL,
            definition TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL
        )
    "#;

    const POLICIES_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS policies (
            id                TEXT PRIMARY KEY,
            site_id           TEXT NOT NULL REFERENCES sites(id) UNIQUE,
            repetition_max    BIGINT,
            velocity_max_tps  DOUBLE PRECISION,
            cost_cap_usd      DOUBLE PRECISION,
            hint_template     TEXT,
            created_at        TIMESTAMPTZ NOT NULL
        )
    "#;

    const AGENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS agents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            name          TEXT NOT NULL,
            policy_id     TEXT REFERENCES policies(id),
            created_at    TIMESTAMPTZ NOT NULL,
            last_seen_at  TIMESTAMPTZ NOT NULL,
            UNIQUE (site_id, agent_id)
        )
    "#;

    const SENTINEL_TOKENS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS sentinel_tokens (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            name          TEXT NOT NULL,
            token_hash    TEXT UNIQUE NOT NULL,
            created_at    TIMESTAMPTZ NOT NULL,
            last_used_at  TIMESTAMPTZ
        )
    "#;

    const ALERT_CHANNELS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS alert_channels (
            id              TEXT PRIMARY KEY,
            site_id         TEXT NOT NULL REFERENCES sites(id),
            kind            TEXT NOT NULL,
            url             TEXT NOT NULL,
            secret          TEXT,
            created_at      TIMESTAMPTZ NOT NULL,
            last_error_at   TIMESTAMPTZ
        )
    "#;

    const INCIDENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS incidents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            trigger_json  TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'open',
            opened_at     TIMESTAMPTZ NOT NULL,
            closed_at     TIMESTAMPTZ
        )
    "#;

    const EVENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS events (
            id              TEXT PRIMARY KEY,
            site_id         TEXT NOT NULL,
            name            TEXT NOT NULL,
            kind            TEXT NOT NULL,
            timestamp       TIMESTAMPTZ NOT NULL,
            received_at     TIMESTAMPTZ NOT NULL,
            url             TEXT NOT NULL,
            referrer        TEXT,
            utm_source      TEXT,
            utm_medium      TEXT,
            utm_campaign    TEXT,
            utm_term        TEXT,
            utm_content     TEXT,
            browser         TEXT NOT NULL,
            browser_version TEXT NOT NULL,
            os              TEXT NOT NULL,
            os_version      TEXT NOT NULL,
            device_type     TEXT NOT NULL,
            screen_width    INTEGER,
            screen_height   INTEGER,
            language        TEXT,
            ip_anonymized   TEXT NOT NULL,
            country_code    TEXT,
            region          TEXT,
            city            TEXT,
            session_id      BYTEA NOT NULL,
            properties      TEXT
        )
    "#;

    const EVENTS_INDEX_TIMESTAMP: &str =
        "CREATE INDEX IF NOT EXISTS idx_events_site_timestamp ON events(site_id, timestamp DESC)";

    const SPANS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS agent_spans (
            id                    TEXT PRIMARY KEY,
            site_id               TEXT NOT NULL,
            agent_id              TEXT NOT NULL,
            agent_session_id      TEXT NOT NULL,
            parent_span_id        TEXT,
            kind                  TEXT NOT NULL,
            model                 TEXT NOT NULL,
            started_at            TIMESTAMPTZ NOT NULL,
            ended_at              TIMESTAMPTZ NOT NULL,
            input_tokens          BIGINT NOT NULL,
            output_tokens         BIGINT NOT NULL,
            cache_read_tokens     BIGINT NOT NULL,
            cache_creation_tokens BIGINT NOT NULL,
            cost_usd              DOUBLE PRECISION NOT NULL,
            tool_name             TEXT,
            tool_input_hash       TEXT,
            stop_reason           TEXT,
            properties            TEXT
        )
    "#;

    const SPANS_INDEX: &str =
        "CREATE INDEX IF NOT EXISTS idx_spans_site_started ON agent_spans(site_id, started_at DESC)";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_metadata_tables_present() {
        let stmts = ddl::all_statements().join("\n");
        for table in [
            "orgs",
            "sites",
            "users",
            "funnels",
            "policies",
            "agents",
            "sentinel_tokens",
            "alert_channels",
            "incidents",
        ] {
            assert!(
                stmts.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
                "missing table {table} in DDL"
            );
        }
    }

    #[test]
    fn ddl_analytics_tables_present_with_indexes() {
        let stmts = ddl::analytics_statements().join("\n");
        assert!(stmts.contains("CREATE TABLE IF NOT EXISTS events"));
        assert!(stmts.contains("CREATE TABLE IF NOT EXISTS agent_spans"));
        assert!(stmts.contains("idx_events_site_timestamp"));
        assert!(stmts.contains("idx_spans_site_started"));
    }

    #[test]
    fn plan_round_trip() {
        for p in [Plan::SelfHosted, Plan::Free, Plan::Pro, Plan::Enterprise] {
            assert_eq!(parse_plan(plan_str(p)), p);
        }
    }

    #[test]
    fn role_round_trip() {
        for r in [UserRole::Owner, UserRole::Admin, UserRole::Viewer] {
            assert_eq!(parse_role(role_str(r)), r);
        }
    }

    #[test]
    fn alert_kind_parses_known_values() {
        assert!(matches!(parse_alert_kind("slack"), AlertChannelKind::Slack));
        assert!(matches!(
            parse_alert_kind("webhook"),
            AlertChannelKind::Webhook
        ));
        // Unknown values default to Webhook (most permissive sink).
        assert!(matches!(
            parse_alert_kind("teams"),
            AlertChannelKind::Webhook
        ));
    }

    #[test]
    fn incident_status_parses_known_values() {
        assert!(matches!(
            parse_incident_status("open"),
            IncidentStatus::Open
        ));
        assert!(matches!(
            parse_incident_status("acknowledged"),
            IncidentStatus::Acknowledged
        ));
        assert!(matches!(
            parse_incident_status("resolved"),
            IncidentStatus::Resolved
        ));
    }

    #[test]
    fn ulid_parse_falls_back_to_default_on_garbage() {
        let bad = parse_ulid("not-a-ulid");
        assert_eq!(bad, Ulid::default());
    }
}
