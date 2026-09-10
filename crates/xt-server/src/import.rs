use serde::Deserialize;
use serde_json::{Value, json};
use xt_ingest::{
    canonical::{Parsed, SourceContext, parse_with_context},
    writer::{ChangeEvent, MAX_BATCH_RECORDS, WriteBatch, resolve_session, write_batch},
};
use xt_store::{SessionSource, Store, ingest::CaptureReceipt};

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Flush {
    Auto,
    Now,
    Defer,
}

#[derive(Deserialize)]
pub(crate) struct ImportArgs {
    messages: Vec<Value>,
    conversation_id: Option<String>,
    source_platform: String,
    source_surface: Option<String>,
    native_session_id: Option<String>,
    title: Option<String>,
    namespace: Option<String>,
    #[serde(rename = "flush")]
    _flush: Option<Flush>,
    // Cloud routing and provenance fields are forward-compatible inputs, not
    // local routing authority. Unsupported provenance is never acknowledged.
}

fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512
}

pub(crate) struct ImportOutcome {
    pub result: Result<Value, &'static str>,
    pub events: Vec<ChangeEvent>,
}

pub(crate) fn apply(store: &mut Store, args: ImportArgs) -> Result<ImportOutcome, &'static str> {
    if args.messages.is_empty() || args.messages.len() > MAX_BATCH_RECORDS {
        return Err("Import requires between 1 and 2000 messages");
    }
    if !label(&args.source_platform)
        || [
            &args.conversation_id,
            &args.source_surface,
            &args.native_session_id,
            &args.namespace,
        ]
        .into_iter()
        .flatten()
        .any(|value| !label(value))
    {
        return Err("Import identity labels must be nonempty and bounded");
    }
    let conversation_id = args
        .conversation_id
        .unwrap_or_else(|| format!("agent-{}", uuid::Uuid::new_v4()));
    let context = SourceContext {
        conversation_id: Some(conversation_id.clone()),
        source_platform: Some(args.source_platform),
        source_surface: args.source_surface,
        native_session_id: args.native_session_id,
        source: Some(SessionSource::Plugin),
        ..Default::default()
    };
    let received = args.messages.len();
    let mut records = Vec::new();
    let mut dropped = 0;
    for message in args.messages {
        match parse_with_context(&message.to_string(), &context)
            .map_err(|_| "Import contains a malformed canonical record")?
        {
            Parsed::Record(record) => records.push(*record),
            Parsed::Dropped(_) => dropped += 1,
            // The endpoint accepts canonical message records. Native structural
            // summaries and PR-link observations retain their dedicated consumers.
            Parsed::Inert | Parsed::StructuralEvent(_) | Parsed::PrLink(_) => {}
        }
    }
    if records.is_empty() {
        return Err("Import contains no eligible canonical records");
    }
    let observed_at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Import clock is unavailable")?
            .as_millis(),
    )
    .map_err(|_| "Import clock is unavailable")?;
    let mut batch = WriteBatch {
        context: &context,
        declared_host: None,
        records: &records,
        title: args.title.as_deref(),
        namespace: args.namespace.as_deref(),
        keep_content: true,
        observed_at,
        receipt: None,
        cursor: None,
    };
    let session = resolve_session(&batch).map_err(|_| "Import session identities disagree")?;
    if !label(&session.session_id)
        || [
            &session.source_platform,
            &session.surface,
            &session.native_session_id,
        ]
        .into_iter()
        .flatten()
        .any(|value| !label(value))
    {
        return Err("Import identity labels must be nonempty and bounded");
    }
    let receipt = CaptureReceipt {
        receipt_id: uuid::Uuid::new_v4().to_string(),
        session_id: session.session_id,
        surface: session.surface,
        received_at: observed_at,
    };
    batch.receipt = Some(&receipt);
    let saved = write_batch(store, &batch).map_err(|_| "Import could not be committed")?;
    // A wholly rejected batch can still commit a conflict on its original owner.
    // Preserve those invalidations even though there is no successful import ack.
    let result = saved.ack_through
        .ok_or("Import contained no accepted record identities")
        .map(|ack| json!({
            "conversation_id": conversation_id, "path":"agentic", "messages_received":received,
            "records_new":saved.records_new, "records_enriched":saved.records_enriched,
            "records_dropped":saved.records_dropped + dropped, "ack_through":ack,
            "pending":0, "draining":false, "provenance_received":{"github_pr_urls":[]},
            "scope":{"source":"local","agent_brain_id":null,"org_name":"local","workspace_name":"local"}
        }));
    Ok(ImportOutcome {
        result,
        events: saved.events,
    })
}

pub(crate) fn schema() -> Value {
    json!({"name":"import_conversation","description":"Save canonical coding-session messages locally and acknowledge only after commit.","inputSchema":{
        "type":"object","properties":{
            "messages":{"type":"array","minItems":1,"maxItems":2000,"items":{"type":"object"}},
            "conversation_id":{"type":"string"},"source_platform":{"type":"string"},
            "source_surface":{"type":"string"},"native_session_id":{"type":"string"},
            "flush":{"type":"string","enum":["auto","now","defer"]},
            "namespace":{"type":"string"},"title":{"type":"string"},
            "provenance":{"type":"object"},"agent_brain_id":{"type":"string"},"org_id":{"type":"string"}
        },"required":["messages","source_platform"]
    }})
}
