use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};
use tokiori_model::{Model, ValidTime, canonical_json, parse_model_str};
use tokiori_pg::{ApplyOutcome, EntityRow, NewEntity, NewRelation, PgError, PgStore, RelationRow};

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
