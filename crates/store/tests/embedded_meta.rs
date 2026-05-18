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
