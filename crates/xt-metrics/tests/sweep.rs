use jiff::Timestamp;
use serde_json::json;
use std::path::PathBuf;
use xt_fixtures::{Fixture, TempDb};
use xt_metrics::{Concurrency, MetricsDb, Window};
use xt_store::CanonicalRecord;

fn fixture() -> Fixture {
    Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F2")).unwrap()
}
fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn window() -> Window {
    Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z")).unwrap()
}

#[test]
fn sweep_weighted_concurrency_golden_and_reversed_arrival() {
    let f = fixture();
    let spans = &f.snapshots()["spans"];
    let golden = &f.snapshots()["concurrency"];
    let base = ms("2026-09-07T12:00:00Z");
    for reverse in [false, true] {
        let mut db = TempDb::empty().unwrap();
        let mut lanes = spans["lanes"].as_array().unwrap().clone();
        if reverse {
            lanes.reverse();
        }
        for lane in lanes {
            let mut session = f.sessions()[0].metadata.clone();
            session.session_id = lane["session_id"].as_str().unwrap().into();
            db.store_mut().upsert_session(&session, false).unwrap();
            let mut records: Vec<CanonicalRecord> =
                serde_json::from_value(lane["records"].clone()).unwrap();
            if reverse {
                records.reverse();
            }
            db.store_mut()
                .upsert_records(&session.session_id, &records, false)
                .unwrap();
        }
        let metrics = MetricsDb::open(db.path()).unwrap();
        let actual = metrics.concurrency(window()).unwrap();
        assert_eq!(serde_json::to_value(&actual).unwrap(), golden["expected"]);
        let active = metrics.active_spans(window()).unwrap();
        let mut weighted = 0;
        let mut wall = 0;
        for interval in golden["occupancy_intervals"].as_array().unwrap() {
            let start = base + interval["start_minute"].as_i64().unwrap() * 60000;
            let end = base + interval["end_minute"].as_i64().unwrap() * 60000;
            let k = interval["occupancy"].as_u64().unwrap();
            assert_eq!(
                active
                    .spans
                    .iter()
                    .filter(|s| s.start_ms <= start && s.end_ms >= end)
                    .count() as u64,
                k
            );
            weighted += (end - start) as u64 * k;
            wall += (end - start) as u64;
        }
        assert_eq!(weighted, active.active_ms);
        assert_eq!(wall, actual.wall_active_ms);
        assert_eq!(Some(weighted as f64 / wall as f64), actual.mean);
    }
}

#[test]
fn sweep_database_empty_singletons_and_touching_lanes() {
    let f = fixture();
    let mut db = TempDb::empty().unwrap();
    let metrics = MetricsDb::open(db.path()).unwrap();
    let empty = Concurrency {
        max: None,
        mean: None,
        wall_active_ms: 0,
    };
    assert_eq!(metrics.concurrency(window()).unwrap(), empty);
    for (id, times) in [
        ("zero", vec!["12:00:00", "12:00:00"]),
        ("one", vec!["12:10:00"]),
        ("left", vec!["12:00:00", "12:10:00"]),
        ("right", vec!["12:10:00", "12:20:00"]),
    ] {
        let mut session = f.sessions()[0].metadata.clone();
        session.session_id = id.into();
        db.store_mut().upsert_session(&session, false).unwrap();
        let rows:Vec<CanonicalRecord>=times.iter().enumerate().map(|(i,time)|serde_json::from_value(json!({"uuid":format!("{id}-{i}"),"type":"assistant","timestamp":format!("2026-09-07T{time}Z"),"message":{"role":"assistant"}})).unwrap()).collect();
        db.store_mut().upsert_records(id, &rows, false).unwrap();
        if id == "zero" || id == "one" {
            assert_eq!(metrics.concurrency(window()).unwrap(), empty);
        }
    }
    assert_eq!(
        metrics.concurrency(window()).unwrap(),
        Concurrency {
            max: Some(1),
            mean: Some(1.0),
            wall_active_ms: 1200000
        }
    );
    let narrow = Window::new(ms("2026-09-07T12:00:00Z"), ms("2026-09-07T12:10:00Z")).unwrap();
    assert_eq!(metrics.concurrency(narrow).unwrap(), empty);
}
