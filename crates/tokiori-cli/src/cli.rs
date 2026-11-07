use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};
use tokiori_model::{
    EntityRef, Hop, Limits, Model, Query, Select, ValidTime, canonical_json, parse_model_str,
};
use tokiori_pg::{
    ApplyOutcome, CancelTransfer, Cutover, EntityRow, NewEntity, NewRelation, OrgChart, PgError,
    PgStore, ReadAttribute, ReadRole, RelationRow, SetAttribute, SetRole, Transfer,
};

#[derive(Debug, Parser)]
#[command(
    name = "tokiori",
    about = "Temporal and relational data engine for PostgreSQL",
    version
)]
pub struct Cli {
    /// PostgreSQL connection string. Falls back to `DATABASE_URL`.
    #[arg(long, global = true, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// Data scope every command reads and writes.
    #[arg(long, global = true, default_value = "default")]
    scope: String,
    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate or apply a model definition.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Create or read an entity.
    Entity {
        #[command(subcommand)]
        action: EntityAction,
    },
    /// Create or read a relation.
    Relation {
        #[command(subcommand)]
        action: RelationAction,
    },
    /// Record or read an attribute as of a date and revision.
    Attribute {
        #[command(subcommand)]
        action: AttributeAction,
    },
    /// Record or read who fills a role.
    Role {
        #[command(subcommand)]
        action: RoleAction,
    },
    /// Run a business operation such as a transfer.
    Operation {
        #[command(subcommand)]
        action: OperationAction,
    },
    /// Deliver events written with operations.
    Outbox {
        #[command(subcommand)]
        action: OutboxAction,
    },
    /// Query relationships as of a date and revision.
    Query {
        #[command(subcommand)]
        action: QueryAction,
    },
    /// Import P0's current state as the first revision.
    Cutover {
        /// The date after which imported facts are asserted.
        #[arg(long)]
        at: NaiveDate,
    },
}

#[derive(Debug, Args)]
struct QueryArgs {
    /// The entity type to start from.
    #[arg(long = "from-type")]
    start_type: String,
    /// The entity id to start from.
    #[arg(long = "from")]
    id: String,
    /// The relation to traverse.
    #[arg(long)]
    relation: String,
    /// One hop as `ROLE=NEXT_ROLE`. Repeat for a longer path.
    #[arg(long = "path")]
    path: Vec<String>,
    /// Return this attribute of the reached entity instead of its id.
    #[arg(long)]
    attribute: Option<String>,
    #[arg(long)]
    valid_at: NaiveDate,
    #[arg(long)]
    known_revision: Option<u64>,
    #[arg(long, default_value_t = 4)]
    max_depth: usize,
    #[arg(long, default_value_t = 1000)]
    max_rows: usize,
}

#[derive(Debug, Args)]
struct OrgChartArgs {
    #[arg(long)]
    valid_at: NaiveDate,
    #[arg(long)]
    known_revision: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum QueryAction {
    /// Run a typed query over relationships.
    Run(QueryArgs),
    /// List every employee and the department they were in.
    OrgChart(OrgChartArgs),
}
#[derive(Debug, Args)]
struct TransferArgs {
    #[arg(long)]
    employee: String,
    #[arg(long)]
    to_department: String,
    #[arg(long)]
    effective_on: NaiveDate,
    /// The caller's request id; replaying it is a no-op.
    #[arg(long)]
    request_id: String,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    actor: String,
}

#[derive(Debug, Args)]
struct CancelArgs {
    /// The request id of the transfer to undo.
    #[arg(long)]
    transfer_request_id: String,
    /// The caller's request id for the cancellation.
    #[arg(long)]
    cancel_request_id: String,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    actor: String,
}

#[derive(Debug, Subcommand)]
enum OperationAction {
    /// Move an employee to another department.
    Transfer(TransferArgs),
    /// Undo a transfer by compensating for it.
    Cancel(CancelArgs),
}

#[derive(Debug, Subcommand)]
enum OutboxAction {
    /// Claim pending events for delivery.
    Claim {
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// Acknowledge events a consumer has processed.
    Ack {
        #[arg(long = "event-id", required = true)]
        event_ids: Vec<String>,
    },
}
#[derive(Debug, Subcommand)]
enum RoleAction {
    /// Record a participant, closing the version it replaces.
    Set {
        #[arg(long = "type")]
        relation_type: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        role: String,
        /// The participant entity id.
        #[arg(long)]
        participant: String,
        #[arg(long)]
        valid_from: NaiveDate,
        #[arg(long)]
        valid_until: Option<NaiveDate>,
    },
    /// Read who filled a role as of a date and revision.
    Get {
        #[arg(long = "type")]
        relation_type: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        valid_at: NaiveDate,
        #[arg(long)]
        known_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
enum AttributeAction {
    /// Record a value, closing the version it replaces.
    Set {
        #[arg(long = "type")]
        owner_type: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        attribute: String,
        #[arg(long)]
        value: String,
        #[arg(long)]
        valid_from: NaiveDate,
        #[arg(long)]
        valid_until: Option<NaiveDate>,
    },
    /// Read a value as of a date and revision.
    Get {
        #[arg(long = "type")]
        owner_type: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        attribute: String,
        /// The business date to ask about.
        #[arg(long)]
        valid_at: NaiveDate,
        /// The revision to answer as of; defaults to the newest.
        #[arg(long)]
        known_revision: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
enum ModelAction {
    /// Parse and validate a model file without touching the database.
    Validate {
        #[arg(long)]
        file: PathBuf,
    },
    /// Store a model version, or confirm an identical one is already stored.
    Apply {
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum EntityAction {
    /// Create an entity.
    Create {
        #[arg(long = "type")]
        entity_type: String,
        #[arg(long)]
        id: String,
        #[arg(long, conflicts_with = "attributes_file")]
        attributes: Option<String>,
        #[arg(long)]
        attributes_file: Option<PathBuf>,
    },
    /// Read an entity.
    Get {
        #[arg(long = "type")]
        entity_type: String,
        #[arg(long)]
        id: String,
    },
}

#[derive(Debug, Args)]
struct RelationCreateArgs {
    #[arg(long = "type")]
    relation_type: String,
    #[arg(long)]
    id: String,
    /// Role participant as `ROLE=ENTITY_ID`. Repeat for each role.
    #[arg(long = "role", required = true)]
    roles: Vec<String>,
    #[arg(long, conflicts_with = "attributes_file")]
    attributes: Option<String>,
    #[arg(long)]
    attributes_file: Option<PathBuf>,
    /// First day the relation is effective.
    #[arg(long)]
    valid_from: NaiveDate,
    /// First day the relation is no longer effective (exclusive).
    #[arg(long)]
    valid_until: Option<NaiveDate>,
}

#[derive(Debug, Subcommand)]
enum RelationAction {
    /// Create a relation.
    Create(RelationCreateArgs),
    /// Read a relation.
    Get {
        #[arg(long = "type")]
        relation_type: String,
        #[arg(long)]
        id: String,
    },
}

/// A failure reported to the operator.
pub struct Failure {
    code: &'static str,
    message: String,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Writes the machine-readable error to stderr.
    pub fn report(&self) {
        let payload = json!({ "code": self.code, "message": self.message });
        eprintln!("{payload}");
    }
}

impl From<PgError> for Failure {
    fn from(err: PgError) -> Self {
        Self::new(err.code(), err.to_string())
    }
}

impl From<std::io::Error> for Failure {
    fn from(err: std::io::Error) -> Self {
        Self::new("io_error", err.to_string())
    }
}

pub async fn run() -> Result<(), Failure> {
    let Cli {
        database_url,
        scope,
        format,
        command,
    } = Cli::parse();
    let ctx = Context {
        database_url: database_url.as_deref(),
        scope: &scope,
        format,
    };

    // The commands split into two groups so each dispatch stays readable:
    // the P0 surface, and the temporal and operational surface added later.
    match command {
        Command::Model { action } => match action {
            ModelAction::Validate { file } => validate(ctx, &file),
            ModelAction::Apply { file } => apply(ctx, &file).await,
        },
        Command::Entity { action } => match action {
            EntityAction::Create {
                entity_type,
                id,
                attributes,
                attributes_file,
            } => {
                let attributes = read_attributes(attributes, attributes_file)?;
                entity_create(ctx, entity_type, id, attributes).await
            }
            EntityAction::Get { entity_type, id } => entity_get(ctx, &entity_type, &id).await,
        },
        Command::Relation { action } => match action {
            RelationAction::Create(args) => relation_create(ctx, args).await,
            RelationAction::Get { relation_type, id } => {
                relation_get(ctx, &relation_type, &id).await
            }
        },
        other => run_temporal(ctx, other).await,
    }
}

/// Dispatches the commands introduced after P0.
async fn run_temporal(ctx: Context<'_>, command: Command) -> Result<(), Failure> {
    match command {
        Command::Attribute { action } => match action {
            AttributeAction::Set {
                owner_type,
                id,
                attribute,
                value,
                valid_from,
                valid_until,
            } => {
                attribute_set(
                    ctx,
                    owner_type,
                    id,
                    attribute,
                    value,
                    valid_from,
                    valid_until,
                )
                .await
            }
            AttributeAction::Get {
                owner_type,
                id,
                attribute,
                valid_at,
                known_revision,
            } => attribute_get(ctx, owner_type, id, attribute, valid_at, known_revision).await,
        },
        Command::Role { action } => match action {
            RoleAction::Set {
                relation_type,
                id,
                role,
                participant,
                valid_from,
                valid_until,
            } => {
                role_set(
                    ctx,
                    relation_type,
                    id,
                    role,
                    participant,
                    valid_from,
                    valid_until,
                )
                .await
            }
            RoleAction::Get {
                relation_type,
                id,
                role,
                valid_at,
                known_revision,
            } => role_get(ctx, relation_type, id, role, valid_at, known_revision).await,
        },
        Command::Operation { action } => match action {
            OperationAction::Transfer(args) => operation_transfer(ctx, args).await,
            OperationAction::Cancel(args) => operation_cancel(ctx, args).await,
        },
        Command::Outbox { action } => match action {
            OutboxAction::Claim { limit } => outbox_claim(ctx, limit).await,
            OutboxAction::Ack { event_ids } => outbox_ack(ctx, &event_ids).await,
        },
        Command::Query { action } => match action {
            QueryAction::Run(args) => query_run(ctx, args).await,
            QueryAction::OrgChart(args) => query_org_chart(ctx, args).await,
        },
        Command::Cutover { at } => cutover(ctx, at).await,
        // Handled by the caller.
        Command::Model { .. } | Command::Entity { .. } | Command::Relation { .. } => {
            unreachable!("P0 commands are dispatched by `run`")
        }
    }
}

/// The connection and output settings shared by every command.
#[derive(Clone, Copy)]
struct Context<'a> {
    database_url: Option<&'a str>,
    scope: &'a str,
    format: Format,
}

fn validate(ctx: Context, path: &PathBuf) -> Result<(), Failure> {
    let model = load_model(path)?;
    emit(ctx.format, &model_summary(&model), || {
        format!(
            "model {} version {} is valid (time zone {})",
            model.model, model.version, model.time_zone
        )
    });
    Ok(())
}

async fn apply(ctx: Context<'_>, path: &PathBuf) -> Result<(), Failure> {
    let model = load_model(path)?;
    let store = open(ctx.database_url).await?;
    let outcome = store.apply_model(ctx.scope, &model).await?;
    let label = match outcome {
        ApplyOutcome::Created => "created",
        ApplyOutcome::Unchanged => "unchanged",
    };
    emit(
        ctx.format,
        &json!({
            "outcome": label,
            "model": model.model,
            "version": model.version,
            "digest": tokiori_model::model_digest(&model),
            "scope": ctx.scope,
        }),
        || format!("model {} version {}: {label}", model.model, model.version),
    );
    Ok(())
}

async fn entity_create(
    ctx: Context<'_>,
    entity_type: String,
    id: String,
    attributes: Map<String, Value>,
) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let row = store
        .create_entity(NewEntity {
            scope: ctx.scope.to_owned(),
            entity_type,
            id,
            attributes,
        })
        .await?;
    emit(ctx.format, &entity_json(&row), || {
        format!("created {} {}", row.entity_type, row.id)
    });
    Ok(())
}

async fn entity_get(ctx: Context<'_>, entity_type: &str, id: &str) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let row = store
        .get_entity(ctx.scope, entity_type, id)
        .await?
        .ok_or_else(|| Failure::new("not_found", format!("{entity_type} {id} does not exist")))?;
    emit(ctx.format, &entity_json(&row), || {
        format!("{} {}", row.entity_type, row.id)
    });
    Ok(())
}

async fn relation_create(ctx: Context<'_>, args: RelationCreateArgs) -> Result<(), Failure> {
    let roles = parse_roles(&args.roles)?;
    let attributes = read_attributes(args.attributes, args.attributes_file)?;
    let valid_time = match args.valid_until {
        None => ValidTime::unbounded_from(args.valid_from),
        Some(until) => ValidTime::bounded(args.valid_from, until)
            .map_err(|err| Failure::new("empty_interval", err.to_string()))?,
    };
    let store = open(ctx.database_url).await?;
    let row = store
        .create_relation(NewRelation {
            scope: ctx.scope.to_owned(),
            relation_type: args.relation_type,
            id: args.id,
            roles,
            attributes,
            valid_time,
        })
        .await?;
    emit(ctx.format, &relation_json(&row), || {
        format!("created {} {}", row.relation_type, row.id)
    });
    Ok(())
}

async fn relation_get(ctx: Context<'_>, relation_type: &str, id: &str) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let row = store
        .get_relation(ctx.scope, relation_type, id)
        .await?
        .ok_or_else(|| Failure::new("not_found", format!("{relation_type} {id} does not exist")))?;
    emit(ctx.format, &relation_json(&row), || {
        format!("{} {}", row.relation_type, row.id)
    });
    Ok(())
}

async fn attribute_set(
    ctx: Context<'_>,
    owner_type: String,
    id: String,
    attribute: String,
    value: String,
    valid_from: NaiveDate,
    valid_until: Option<NaiveDate>,
) -> Result<(), Failure> {
    let valid_time = match valid_until {
        None => ValidTime::unbounded_from(valid_from),
        Some(until) => ValidTime::bounded(valid_from, until)
            .map_err(|err| Failure::new("empty_interval", err.to_string()))?,
    };
    let store = open(ctx.database_url).await?;
    let revision = store
        .set_attribute(SetAttribute {
            scope: ctx.scope.to_owned(),
            entity_type: owner_type.clone(),
            entity_id: id.clone(),
            attribute: attribute.clone(),
            value: Value::String(value),
            valid_time,
        })
        .await?;
    emit(
        ctx.format,
        &json!({ "revision": revision, "owner_type": owner_type, "id": id, "attribute": attribute }),
        || format!("recorded {owner_type} {id} {attribute} at revision {revision}"),
    );
    Ok(())
}

async fn attribute_get(
    ctx: Context<'_>,
    owner_type: String,
    id: String,
    attribute: String,
    valid_at: NaiveDate,
    known_revision: Option<u64>,
) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let answer = store
        .read_attribute(ReadAttribute {
            scope: ctx.scope.to_owned(),
            entity_type: owner_type,
            entity_id: id,
            attribute,
            valid_at,
            known_revision,
        })
        .await?;
    emit(
        ctx.format,
        &json!({
            "value": answer.value,
            "known_revision": answer.known_revision,
            "valid_at": answer.valid_at,
        }),
        || match &answer.value {
            Some(value) => format!("{value}"),
            None => "(no value)".to_owned(),
        },
    );
    Ok(())
}

async fn role_set(
    ctx: Context<'_>,
    relation_type: String,
    id: String,
    role: String,
    participant: String,
    valid_from: NaiveDate,
    valid_until: Option<NaiveDate>,
) -> Result<(), Failure> {
    let valid_time = match valid_until {
        None => ValidTime::unbounded_from(valid_from),
        Some(until) => ValidTime::bounded(valid_from, until)
            .map_err(|err| Failure::new("empty_interval", err.to_string()))?,
    };
    let store = open(ctx.database_url).await?;
    let revision = store
        .set_role(SetRole {
            scope: ctx.scope.to_owned(),
            relation_type: relation_type.clone(),
            relation_id: id.clone(),
            role: role.clone(),
            participant_id: participant,
            valid_time,
        })
        .await?;
    emit(
        ctx.format,
        &json!({ "revision": revision, "relation_type": relation_type, "id": id, "role": role }),
        || format!("recorded {relation_type} {id} {role} at revision {revision}"),
    );
    Ok(())
}

async fn role_get(
    ctx: Context<'_>,
    relation_type: String,
    id: String,
    role: String,
    valid_at: NaiveDate,
    known_revision: Option<u64>,
) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let answer = store
        .read_role(ReadRole {
            scope: ctx.scope.to_owned(),
            relation_type,
            relation_id: id,
            role,
            valid_at,
            known_revision,
        })
        .await?;
    emit(
        ctx.format,
        &json!({
            "participant": answer.participant_id,
            "known_revision": answer.known_revision,
            "valid_at": answer.valid_at,
        }),
        || match &answer.participant_id {
            Some(value) => value.clone(),
            None => "(no participant)".to_owned(),
        },
    );
    Ok(())
}

async fn operation_transfer(ctx: Context<'_>, args: TransferArgs) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let outcome = store
        .transfer(Transfer {
            scope: ctx.scope.to_owned(),
            employee: args.employee,
            to_department: args.to_department,
            effective_on: args.effective_on,
            request_id: args.request_id,
            reason: args.reason,
            actor: args.actor,
        })
        .await?;
    emit(
        ctx.format,
        &json!({
            "revision": outcome.revision,
            "applied": outcome.applied,
            "result": outcome.result,
        }),
        || {
            format!(
                "transfer {} at revision {}",
                if outcome.applied {
                    "applied"
                } else {
                    "replayed"
                },
                outcome.revision
            )
        },
    );
    Ok(())
}

async fn operation_cancel(ctx: Context<'_>, args: CancelArgs) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let outcome = store
        .cancel_transfer(CancelTransfer {
            scope: ctx.scope.to_owned(),
            transfer_request_id: args.transfer_request_id,
            cancel_request_id: args.cancel_request_id,
            reason: args.reason,
            actor: args.actor,
        })
        .await?;
    emit(
        ctx.format,
        &json!({
            "revision": outcome.revision,
            "applied": outcome.applied,
            "result": outcome.result,
        }),
        || {
            format!(
                "cancel {} at revision {}",
                if outcome.applied {
                    "applied"
                } else {
                    "replayed"
                },
                outcome.revision
            )
        },
    );
    Ok(())
}

async fn outbox_claim(ctx: Context<'_>, limit: i64) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let events = store.claim_outbox(ctx.scope, limit).await?;
    let payload: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "event_id": event.event_id,
                "revision": event.revision,
                "event_type": event.event_type,
                "payload": event.payload,
            })
        })
        .collect();
    let count = payload.len();
    emit(ctx.format, &Value::Array(payload), || {
        format!("{count} event(s) claimed")
    });
    Ok(())
}

async fn outbox_ack(ctx: Context<'_>, event_ids: &[String]) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let mut ids = Vec::with_capacity(event_ids.len());
    for text in event_ids {
        let id = text
            .parse()
            .map_err(|_| Failure::new("invalid_event_id", format!("{text:?} is not a UUID")))?;
        ids.push(id);
    }
    let acknowledged = store.ack_outbox(ctx.scope, &ids).await?;
    emit(ctx.format, &json!({ "acknowledged": acknowledged }), || {
        format!("{acknowledged} event(s) acknowledged")
    });
    Ok(())
}

async fn query_run(ctx: Context<'_>, args: QueryArgs) -> Result<(), Failure> {
    let query = build_query(&args)?;
    let store = open(ctx.database_url).await?;
    let result = store.query(ctx.scope, &query).await?;
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|row| json!({ "id": row.id, "value": row.value }))
        .collect();
    let count = rows.len();
    let truncated = result.truncated;
    emit(
        ctx.format,
        &json!({
            "rows": rows,
            "truncated": truncated,
            "known_revision": result.known_revision,
        }),
        || {
            let suffix = if truncated { " (truncated)" } else { "" };
            format!("{count} row(s){suffix}")
        },
    );
    Ok(())
}

async fn query_org_chart(ctx: Context<'_>, args: OrgChartArgs) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let chart = store
        .org_chart(OrgChart {
            scope: ctx.scope.to_owned(),
            valid_at: args.valid_at,
            known_revision: args.known_revision,
        })
        .await?;
    emit(ctx.format, &json!(chart), || {
        chart
            .iter()
            .map(|row| format!("{} -> {}", row.employee, row.department))
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

/// Turns the command line into a query.
///
/// Each `--path` step is one hop written `ROLE=NEXT_ROLE`; the model decides
/// whether the path is meaningful, so a bad step is a query error rather than
/// a database error.
fn build_query(args: &QueryArgs) -> Result<Query, Failure> {
    let mut traversal = Vec::new();
    for step in &args.path {
        let Some((from_role, to_role)) = step.split_once('=') else {
            return Err(Failure::new(
                "invalid_path",
                format!("path step {step:?} must be written ROLE=NEXT_ROLE"),
            ));
        };
        traversal.push(Hop {
            relation_type: args.relation.clone(),
            from_role: from_role.to_owned(),
            to_role: to_role.to_owned(),
        });
    }
    Ok(Query {
        start: EntityRef {
            entity_type: args.start_type.clone(),
            id: args.id.clone(),
        },
        traversal,
        select: match &args.attribute {
            Some(attribute) => Select::Attribute {
                attribute: attribute.clone(),
            },
            None => Select::Ids,
        },
        valid_at: args.valid_at,
        known_revision: args.known_revision,
        limits: Limits {
            max_depth: args.max_depth,
            max_rows: args.max_rows,
        },
    })
}

async fn cutover(ctx: Context<'_>, at: NaiveDate) -> Result<(), Failure> {
    let store = open(ctx.database_url).await?;
    let outcome = store
        .cutover(Cutover {
            scope: ctx.scope.to_owned(),
            at,
        })
        .await?;
    emit(
        ctx.format,
        &json!({
            "revision": outcome.revision,
            "created": outcome.created,
            "entities": outcome.entities,
            "relations": outcome.relations,
        }),
        || {
            format!(
                "cutover at revision {} ({})",
                outcome.revision,
                if outcome.created {
                    "created"
                } else {
                    "already done"
                }
            )
        },
    );
    Ok(())
}

fn emit(format: Format, value: &Value, text: impl FnOnce() -> String) {
    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(value).expect("json")),
        Format::Text => println!("{}", text()),
    }
}

async fn open(database_url: Option<&str>) -> Result<PgStore, Failure> {
    let Some(url) = database_url else {
        return Err(Failure::new(
            "missing_database_url",
            "pass --database-url or set DATABASE_URL",
        ));
    };
    let pool = tokiori_pg::connect(url).await?;
    tokiori_pg::migrate(&pool).await?;
    Ok(PgStore::new(pool))
}

fn load_model(path: &PathBuf) -> Result<Model, Failure> {
    let text = std::fs::read_to_string(path)?;
    parse_model_str(&text).map_err(|err| Failure::new("invalid_model", err.to_string()))
}

fn model_summary(model: &Model) -> Value {
    json!({
        "model": model.model,
        "version": model.version,
        "time_zone": model.time_zone.to_string(),
        "digest": tokiori_model::model_digest(model),
        "canonical": canonical_json(model),
    })
}

fn read_attributes(
    inline: Option<String>,
    file: Option<PathBuf>,
) -> Result<Map<String, Value>, Failure> {
    let text = match (inline, file) {
        (Some(text), _) => text,
        (None, Some(path)) => std::fs::read_to_string(path)?,
        (None, None) => return Ok(Map::new()),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| Failure::new("invalid_attributes", err.to_string()))?;
    match value {
        Value::Object(map) => Ok(map),
        other => Err(Failure::new(
            "invalid_attributes",
            format!("attributes must be a JSON object, got {other}"),
        )),
    }
}

fn parse_roles(roles: &[String]) -> Result<BTreeMap<String, String>, Failure> {
    let mut out = BTreeMap::new();
    for spec in roles {
        let Some((role, participant)) = spec.split_once('=') else {
            return Err(Failure::new(
                "invalid_role",
                format!("role {spec:?} must be written ROLE=ENTITY_ID"),
            ));
        };
        if role.is_empty() || participant.is_empty() {
            return Err(Failure::new(
                "invalid_role",
                format!("role {spec:?} must be written ROLE=ENTITY_ID"),
            ));
        }
        if out
            .insert(role.to_owned(), participant.to_owned())
            .is_some()
        {
            return Err(Failure::new(
                "invalid_role",
                format!("role {role:?} was given more than once"),
            ));
        }
    }
    Ok(out)
}

fn entity_json(row: &EntityRow) -> Value {
    json!({
        "scope": row.scope,
        "entity_type": row.entity_type,
        "id": row.id,
        "model": row.model,
        "model_version": row.model_version,
        "attributes": row.attributes,
    })
}

fn relation_json(row: &RelationRow) -> Value {
    json!({
        "scope": row.scope,
        "relation_type": row.relation_type,
        "id": row.id,
        "model": row.model,
        "model_version": row.model_version,
        "attributes": row.attributes,
        "valid_from": row.valid_time.from,
        "valid_until": row.valid_time.until,
        "roles": row.roles,
    })
}
