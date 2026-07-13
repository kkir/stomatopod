use std::sync::Arc;

use chrono::Utc;
use ulid::Ulid;

use stomatopod_core::{
    domain::{
        org::{Funnel, Organization, Plan, User, UserRole},
        site::Site,
    },
    traits::MetaStore,
};
use stomatopod_store::embedded::meta::SqliteMeta;

async fn open_meta(dir: &tempfile::TempDir) -> SqliteMeta {
    SqliteMeta::open(&dir.path().join("meta.db")).await.unwrap()
}

fn make_org() -> Organization {
    Organization {
        id: Ulid::new(),
        name: "Test Org".into(),
        slug: format!("org-{}", Ulid::new()),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    }
}

fn make_site(org_id: Ulid) -> Site {
    Site {
        id: Ulid::new(),
        org_id,
        domain: format!("{}.example.com", Ulid::new()),
        name: "Test Site".into(),
        timezone: "UTC".into(),
        public_key: Ulid::new().to_string(),
        created_at: Utc::now(),
        is_active: true,
    }
}

// ---- Org tests ----

#[tokio::test]
async fn create_and_get_org() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let found = meta.get_org(org.id).await.unwrap().unwrap();
    assert_eq!(found.id, org.id);
    assert_eq!(found.name, org.name);
    assert_eq!(found.slug, org.slug);
}

#[tokio::test]
async fn get_org_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let result = meta.get_org(Ulid::new()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn list_orgs_empty_initially() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let orgs = meta.list_orgs().await.unwrap();
    assert!(orgs.is_empty());
}

#[tokio::test]
async fn list_orgs_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    meta.create_org(&make_org()).await.unwrap();
    meta.create_org(&make_org()).await.unwrap();
    meta.create_org(&make_org()).await.unwrap();

    let orgs = meta.list_orgs().await.unwrap();
    assert_eq!(orgs.len(), 3);
}

#[tokio::test]
async fn org_slug_must_be_unique() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let mut org1 = make_org();
    org1.slug = "duplicate-slug".into();
    meta.create_org(&org1).await.unwrap();

    let mut org2 = make_org();
    org2.slug = "duplicate-slug".into();
    let result = meta.create_org(&org2).await;
    assert!(result.is_err());
}

// ---- Site tests ----

#[tokio::test]
async fn create_and_get_site() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    let found = meta.get_site(site.id).await.unwrap().unwrap();
    assert_eq!(found.id, site.id);
    assert_eq!(found.domain, site.domain);
    assert_eq!(found.public_key, site.public_key);
    assert!(found.is_active);
}

#[tokio::test]
async fn create_site_writes_default_analytics_alerts() {
    use stomatopod_core::domain::analytics_alert::AnalyticsAlertKind;

    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    let alerts = meta.list_analytics_alerts(site.id).await.unwrap();
    assert_eq!(alerts.len(), 3, "starter rules must be real DB rows");
    assert!(alerts.iter().all(|a| a.enabled && a.site_id == site.id));
    let kinds: Vec<_> = alerts.iter().map(|a| a.kind).collect();
    assert!(kinds.contains(&AnalyticsAlertKind::TrafficSpike));
    assert!(kinds.contains(&AnalyticsAlertKind::TrafficDrop));
    assert!(kinds.contains(&AnalyticsAlertKind::NewReferrerSpike));

    // Users can remove them permanently (no re-seed on next list).
    for a in &alerts {
        meta.delete_analytics_alert(a.id).await.unwrap();
    }
    assert!(meta
        .list_analytics_alerts(site.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn get_site_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let result = meta.get_site(Ulid::new()).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn get_site_by_key() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    let found = meta
        .get_site_by_key(&site.public_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, site.id);

    let missing = meta.get_site_by_key("no-such-key").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn get_site_by_domain() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    let found = meta
        .get_site_by_domain(&site.domain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, site.id);

    let missing = meta.get_site_by_domain("no-such-domain.com").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn list_sites_for_org() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let other_org = make_org();
    meta.create_org(&other_org).await.unwrap();

    meta.create_site(&make_site(org.id)).await.unwrap();
    meta.create_site(&make_site(org.id)).await.unwrap();
    meta.create_site(&make_site(other_org.id)).await.unwrap();

    let sites = meta.list_sites(org.id).await.unwrap();
    assert_eq!(sites.len(), 2);
    assert!(sites.iter().all(|s| s.org_id == org.id));
}

#[tokio::test]
async fn delete_site_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    meta.delete_site(site.id).await.unwrap();

    let result = meta.get_site(site.id).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn update_site_updates_fields() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let mut site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    site.name = "Updated Site Name".into();
    site.domain = "updated.example.com".into();
    site.timezone = "America/New_York".into();
    site.is_active = false;
    meta.update_site(&site).await.unwrap();

    let found = meta.get_site(site.id).await.unwrap().unwrap();
    assert_eq!(found.name, "Updated Site Name");
    assert_eq!(found.domain, "updated.example.com");
    assert_eq!(found.timezone, "America/New_York");
    assert!(!found.is_active);
}

#[tokio::test]
async fn inactive_site_not_returned_by_key_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let mut site = make_site(org.id);
    site.is_active = false;
    meta.create_site(&site).await.unwrap();

    let result = meta.get_site_by_key(&site.public_key).await.unwrap();
    assert!(result.is_none(), "inactive site should not be found by key");
}

// ---- User tests ----

#[tokio::test]
async fn create_and_get_user_by_email() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let user = User {
        id: Ulid::new(),
        org_id: org.id,
        email: "alice@example.com".into(),
        password_hash: "$argon2id$v=19$m=65536,t=2,p=1$fakesalt$fakehash".into(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    meta.create_user(&user).await.unwrap();

    let found = meta
        .get_user_by_email("alice@example.com")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, user.id);
    assert_eq!(found.email, user.email);
    assert_eq!(found.org_id, org.id);
}

#[tokio::test]
async fn get_user_by_email_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let result = meta.get_user_by_email("ghost@example.com").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn user_email_must_be_unique() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();

    let make_user = |email: &str| User {
        id: Ulid::new(),
        org_id: org.id,
        email: email.into(),
        password_hash: "hash".into(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };

    meta.create_user(&make_user("dup@example.com"))
        .await
        .unwrap();
    let result = meta.create_user(&make_user("dup@example.com")).await;
    assert!(result.is_err());
}

// ---- Funnel tests ----

#[tokio::test]
async fn create_list_and_get_funnel() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    assert!(meta.list_funnels(site.id).await.unwrap().is_empty());

    let funnel = Funnel {
        id: Ulid::new(),
        site_id: site.id,
        name: "Signup Funnel".into(),
        definition: r#"[{"event":"pageview"},{"event":"signup"}]"#.into(),
        created_at: Utc::now(),
    };
    meta.create_funnel(&funnel).await.unwrap();

    let funnels = meta.list_funnels(site.id).await.unwrap();
    assert_eq!(funnels.len(), 1);
    assert_eq!(funnels[0].name, "Signup Funnel");

    let found = meta.get_funnel(funnel.id).await.unwrap().unwrap();
    assert_eq!(found.id, funnel.id);
    assert_eq!(found.definition, funnel.definition);
}

#[tokio::test]
async fn delete_funnel_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    let funnel = Funnel {
        id: Ulid::new(),
        site_id: site.id,
        name: "To Delete".into(),
        definition: "[]".into(),
        created_at: Utc::now(),
    };
    meta.create_funnel(&funnel).await.unwrap();

    meta.delete_funnel(funnel.id).await.unwrap();

    assert!(meta.list_funnels(site.id).await.unwrap().is_empty());
    assert!(meta.get_funnel(funnel.id).await.unwrap().is_none());
}

#[tokio::test]
async fn funnels_are_scoped_to_site() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;

    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site_a = make_site(org.id);
    let site_b = make_site(org.id);
    meta.create_site(&site_a).await.unwrap();
    meta.create_site(&site_b).await.unwrap();

    let funnel_a = Funnel {
        id: Ulid::new(),
        site_id: site_a.id,
        name: "Funnel A".into(),
        definition: "[]".into(),
        created_at: Utc::now(),
    };
    meta.create_funnel(&funnel_a).await.unwrap();

    let funnels_a = meta.list_funnels(site_a.id).await.unwrap();
    let funnels_b = meta.list_funnels(site_b.id).await.unwrap();
    assert_eq!(funnels_a.len(), 1);
    assert!(funnels_b.is_empty());
}

// ---- EmbeddedBackend ingest smoke test ----

#[tokio::test]
async fn embedded_backend_accepts_event_batch() {
    use stomatopod_core::{
        config::EmbeddedConfig,
        domain::event::{DeviceType, Event, EventKind},
        traits::StorageBackend,
    };
    use stomatopod_store::embedded::EmbeddedBackend;

    let dir = tempfile::tempdir().unwrap();
    let cfg = EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 0,
        parquet_flush_rows: 10,
        parquet_flush_interval_s: 60,
        allow_ephemeral: true,
    };
    let backend = Arc::new(EmbeddedBackend::open(&cfg).await.unwrap());

    let org = stomatopod_core::domain::org::Organization {
        id: Ulid::new(),
        name: "Test".into(),
        slug: "test".into(),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    };
    backend.meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    backend.meta.create_site(&site).await.unwrap();

    let event = Event {
        id: Ulid::new(),
        site_id: site.id,
        name: "pageview".into(),
        kind: EventKind::Pageview,
        timestamp: Utc::now(),
        received_at: Utc::now(),
        url: "https://example.com/".into(),
        referrer: None,
        utm_source: None,
        utm_medium: None,
        utm_campaign: None,
        utm_term: None,
        utm_content: None,
        browser: "Chrome".into(),
        browser_version: "120".into(),
        os: "macOS".into(),
        os_version: "14".into(),
        device_type: DeviceType::Desktop,
        screen_width: Some(1920),
        screen_height: Some(1080),
        language: Some("en-US".into()),
        ip_anonymized: "1.2.3.0".into(),
        country_code: None,
        region: None,
        city: None,
        session_id: [0u8; 16],
        properties: None,
    };

    backend.ingest_events(vec![event]).await.unwrap();
}

// ---- API key tests ----

#[tokio::test]
async fn api_key_crud_round_trip() {
    use stomatopod_core::domain::api_key::{ApiKey, ApiKeyScope};

    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();

    // One org-wide read key, one site-bound ingest key.
    let (read_key, read_plain) = ApiKey::new_read(org.id, None, "agent".into());
    let (ingest_key, ingest_plain) = ApiKey::new_ingest(org.id, site.id, "backend".into());
    meta.create_api_key(&read_key).await.unwrap();
    meta.create_api_key(&ingest_key).await.unwrap();

    // list is org-scoped and returns both.
    let listed = meta.list_api_keys(org.id).await.unwrap();
    assert_eq!(listed.len(), 2);

    // get_by_hash resolves the right key with site/scope preserved.
    let got_read = meta
        .get_api_key_by_hash(&ApiKey::hash(&read_plain))
        .await
        .unwrap()
        .expect("read key present");
    assert_eq!(got_read.scope, ApiKeyScope::Read);
    assert_eq!(got_read.site_id, None);

    let got_ingest = meta
        .get_api_key_by_hash(&ApiKey::hash(&ingest_plain))
        .await
        .unwrap()
        .expect("ingest key present");
    assert_eq!(got_ingest.scope, ApiKeyScope::Ingest);
    assert_eq!(got_ingest.site_id, Some(site.id));
    assert!(got_ingest.last_used_at.is_none());

    // touch sets last_used_at.
    meta.touch_api_key(ingest_key.id).await.unwrap();
    let touched = meta
        .get_api_key_by_hash(&ApiKey::hash(&ingest_plain))
        .await
        .unwrap()
        .unwrap();
    assert!(touched.last_used_at.is_some());

    // delete removes it.
    meta.delete_api_key(ingest_key.id).await.unwrap();
    assert!(meta
        .get_api_key_by_hash(&ApiKey::hash(&ingest_plain))
        .await
        .unwrap()
        .is_none());
    assert_eq!(meta.list_api_keys(org.id).await.unwrap().len(), 1);
}

// ---- Digest subscriptions ----

use stomatopod_core::domain::digest::{DigestFrequency, DigestSubscription};

/// Create an org + site and return the site.
async fn org_and_site(meta: &SqliteMeta) -> Site {
    let org = make_org();
    meta.create_org(&org).await.unwrap();
    let site = make_site(org.id);
    meta.create_site(&site).await.unwrap();
    site
}

async fn make_persisted_user(meta: &SqliteMeta, site: &Site, email: &str) -> User {
    let user = User {
        id: Ulid::new(),
        org_id: site.org_id,
        email: email.into(),
        password_hash: "x".into(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    };
    meta.create_user(&user).await.unwrap();
    user
}

#[tokio::test]
async fn digest_subscription_upsert_and_get() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let site = org_and_site(&meta).await;
    let user = make_persisted_user(&meta, &site, "d@example.com").await;

    let sub = DigestSubscription {
        id: Ulid::new(),
        user_id: user.id,
        site_id: site.id,
        frequency: DigestFrequency::Weekly,
        enabled: true,
        bounce_count: 0,
        created_at: Utc::now(),
    };
    meta.upsert_digest_subscription(&sub).await.unwrap();

    let got = meta
        .get_digest_subscription(user.id, site.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.frequency, DigestFrequency::Weekly);
    assert!(got.enabled);

    // Upsert again with a new frequency keeps the (user, site) row unique.
    let mut sub2 = sub.clone();
    sub2.id = Ulid::new();
    sub2.frequency = DigestFrequency::Both;
    meta.upsert_digest_subscription(&sub2).await.unwrap();
    let got2 = meta
        .get_digest_subscription(user.id, site.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got2.frequency, DigestFrequency::Both);

    // Enabled list includes it.
    assert_eq!(
        meta.list_enabled_digest_subscriptions()
            .await
            .unwrap()
            .len(),
        1
    );

    // Delete unsubscribes.
    meta.delete_digest_subscription(user.id, site.id)
        .await
        .unwrap();
    assert!(meta
        .get_digest_subscription(user.id, site.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn digest_bounce_disables_after_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let meta = open_meta(&dir).await;
    let site = org_and_site(&meta).await;
    let user = make_persisted_user(&meta, &site, "b@example.com").await;

    let sub = DigestSubscription {
        id: Ulid::new(),
        user_id: user.id,
        site_id: site.id,
        frequency: DigestFrequency::Monthly,
        enabled: true,
        bounce_count: 0,
        created_at: Utc::now(),
    };
    meta.upsert_digest_subscription(&sub).await.unwrap();

    // Two bounces: still enabled.
    meta.record_digest_bounce(sub.id, 3).await.unwrap();
    meta.record_digest_bounce(sub.id, 3).await.unwrap();
    assert_eq!(
        meta.list_enabled_digest_subscriptions()
            .await
            .unwrap()
            .len(),
        1
    );

    // Third bounce crosses the threshold and disables.
    meta.record_digest_bounce(sub.id, 3).await.unwrap();
    assert!(meta
        .list_enabled_digest_subscriptions()
        .await
        .unwrap()
        .is_empty());
}
