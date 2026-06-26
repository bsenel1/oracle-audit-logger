use chrono::{DateTime, Utc};
use glob::glob;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Deserialize;
use sqlx::PgPool;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
struct AppConfig {
    collector: CollectorConfig,
    oracle_file_sources: Vec<OracleFileSourceConfig>,
}

#[derive(Debug, Deserialize, Clone)]
struct CollectorConfig {
    poll_interval_secs: u64,
    sincedb_dir: String,
}

#[derive(Debug, Deserialize, Clone)]
struct OracleFileSourceConfig {
    cluster_name: String,
    server_name: String,
    server_ip: String,
    audit_file_path: String,
}

#[derive(Debug, Default, Clone)]
struct OracleXmlAuditRecord {
    source_file: String,
    extended_timestamp: Option<DateTime<Utc>>,
    db_user: Option<String>,
    os_user: Option<String>,
    userhost: Option<String>,
    terminal: Option<String>,
    object_schema: Option<String>,
    object_name: Option<String>,
    action_code: Option<i32>,
    action_name: String,
    sql_text: Option<String>,
    comment_text: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_config("/app/config.toml")?;

    fs::create_dir_all(&config.collector.sincedb_dir)?;

    let postgres_db_url = env::var("POSTGRES_DB_URL")?;
    let pg_pool = PgPool::connect(&postgres_db_url).await?;

    println!("Oracle 21c XML audit file collector started.");

    loop {
        for source in &config.oracle_file_sources {
            match process_source(&config, source, &pg_pool).await {
                Ok((audit_count, conn_count)) => {
                    println!(
                        "[{} / {}] Oracle XML Audit: {} new audit records, {} new connection records",
                        source.cluster_name, source.server_name, audit_count, conn_count
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[{} / {}] Error: {}",
                        source.cluster_name, source.server_name, err
                    );
                }
            }
        }

        thread::sleep(Duration::from_secs(config.collector.poll_interval_secs));
    }
}

fn load_config(path: &str) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let config: AppConfig = toml::from_str(&content)?;
    Ok(config)
}

async fn process_source(
    config: &AppConfig,
    source: &OracleFileSourceConfig,
    pg_pool: &PgPool,
) -> Result<(u64, u64), Box<dyn std::error::Error>> {
    let files = collect_files(&source.audit_file_path)?;

    let mut inserted_audit_count = 0;
    let mut inserted_connection_count = 0;

    for file_path in files {
        let offset_path = build_file_offset_path(&config.collector.sincedb_dir, source, &file_path);
        let current_size = fs::metadata(&file_path)?.len();
        let last_size = read_file_size_offset(&offset_path)?;

        if current_size <= last_size {
            continue;
        }

        let records = parse_xml_audit_file(&file_path)?;

        for record in records {
            if should_skip_log(&record) {
                continue;
            }

            if is_connection_action(&record.action_name) {
                let inserted = insert_connection_log(pg_pool, source, &record).await?;
                if inserted {
                    inserted_connection_count += 1;
                }
            } else {
                let inserted = insert_audit_log(pg_pool, source, &record).await?;
                if inserted {
                    inserted_audit_count += 1;
                }
            }
        }

        write_file_size_offset(&offset_path, current_size)?;
    }

    Ok((inserted_audit_count, inserted_connection_count))
}

fn collect_files(pattern: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();

    for entry in glob(pattern)? {
        match entry {
            Ok(path) => {
                if path.is_file() {
                    files.push(path);
                }
            }
            Err(err) => eprintln!("Glob error: {}", err),
        }
    }

    files.sort();
    Ok(files)
}

fn parse_xml_audit_file(path: &Path) -> Result<Vec<OracleXmlAuditRecord>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut reader = Reader::from_str(&content);
    reader.trim_text(true);

    let mut records = Vec::new();
    let mut current_record: Option<OracleXmlAuditRecord> = None;
    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();

                if tag == "AuditRecord" {
                    current_record = Some(OracleXmlAuditRecord {
                        source_file: path.to_string_lossy().to_string(),
                        ..Default::default()
                    });
                } else {
                    current_tag = tag;
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(record) = current_record.as_mut() {
                    let value = e.unescape()?.to_string();
                    apply_xml_value(record, &current_tag, &value);
                }
            }
            Ok(Event::End(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();

                if tag == "AuditRecord" {
                    if let Some(mut record) = current_record.take() {
                        record.action_name = action_code_to_name(record.action_code);
                        records.push(record);
                    }
                }

                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(format!("XML parse error {:?}: {}", path, err).into());
            }
            _ => {}
        }
    }

    Ok(records)
}

fn apply_xml_value(record: &mut OracleXmlAuditRecord, tag: &str, value: &str) {
    let cleaned = clean_string(value);

    match tag {
        "Extended_Timestamp" => {
            record.extended_timestamp = parse_oracle_xml_timestamp(&cleaned);
        }
        "DB_User" => record.db_user = non_empty(cleaned),
        "OS_User" => record.os_user = non_empty(cleaned),
        "Userhost" => record.userhost = non_empty(cleaned),
        "Terminal" => record.terminal = non_empty(cleaned),
        "Object_Schema" => record.object_schema = non_empty(cleaned),
        "Object_Name" => record.object_name = non_empty(cleaned),
        "Action" => record.action_code = cleaned.parse::<i32>().ok(),
        "Sql_Text" => record.sql_text = non_empty(cleaned),
        "Comment_Text" => record.comment_text = non_empty(cleaned),
        _ => {}
    }
}

fn parse_oracle_xml_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

fn action_code_to_name(action_code: Option<i32>) -> String {
    match action_code {
        Some(100) => "LOGON".to_string(),
        Some(101) => "LOGOFF".to_string(),
        Some(1) => "CREATE TABLE".to_string(),
        Some(2) => "INSERT".to_string(),
        Some(3) => "SELECT".to_string(),
        Some(6) => "UPDATE".to_string(),
        Some(7) => "DELETE".to_string(),
        Some(12) => "DROP TABLE".to_string(),
        Some(15) => "ALTER TABLE".to_string(),
        Some(code) => format!("ACTION_{}", code),
        None => "UNKNOWN".to_string(),
    }
}

fn should_skip_log(record: &OracleXmlAuditRecord) -> bool {
    if record.extended_timestamp.is_none() {
        return true;
    }

    let db_user = record.db_user.as_deref().unwrap_or("").to_uppercase();
    let object_schema = record.object_schema.as_deref().unwrap_or("").to_uppercase();
    let object_name = record.object_name.as_deref().unwrap_or("").to_uppercase();
    let sql_text = record.sql_text.as_deref().unwrap_or("").to_uppercase();

    if db_user.is_empty() {
        return true;
    }

    if db_user == "/" || db_user == "SYS" || record.action_name == "UNKNOWN" {
        return true;
    }

    // Keep LOGON / LOGOFF records
    if is_connection_action(&record.action_name) {
        return false;
    }

    // Skip system metadata records outside the user schema.
    // Examples: SYS.ALL_CONSTRAINTS, SYS.COL$, SYS.TAB$, SYS.X$KZSRO, etc.
    if object_schema == "SYS" {
        return true;
    }

    // Oracle and DBeaver metadata queries
    if sql_text.contains("ALL_CONSTRAINTS")
        || sql_text.contains("ALL_CONS_COLUMNS")
        || sql_text.contains("ALL_INDEXES")
        || sql_text.contains("ALL_IND_COLUMNS")
        || sql_text.contains("ALL_IND_EXPRESSIONS")
        || sql_text.contains("ALL_TAB_COLS")
        || sql_text.contains("ALL_ALL_TABLES")
        || sql_text.contains("ALL_TABLES")
        || sql_text.contains("USER_OBJECTS")
        || sql_text.contains("DBA_POLICIES")
        || sql_text.contains("XS_SYS_CONTEXT")
        || sql_text.contains("SELECT 1 FROM SYS.")
        || sql_text.contains("SELECT 1 FROM ALL_")
        || sql_text.contains("FROM SYS.DUAL")
    {
        return true;
    }

    // DBeaver table browsing, constraint, and index queries
    if sql_text.contains("WHERE I.TABLE_OWNER")
        || sql_text.contains("WHERE C.OWNER")
        || sql_text.contains("ORDER BY I.TABLE_NAME")
        || sql_text.contains("COLUMN_NAMES_NUMS")
        || sql_text.contains("LISTAGG(COLUMN_NAME")
    {
        return true;
    }

    // Keep real user operations such as INSERT, SELECT, UPDATE, DELETE, CREATE, and DROP.
    false
}

fn is_connection_action(action_name: &str) -> bool {
    matches!(action_name, "LOGON" | "LOGOFF")
}

async fn insert_connection_log(
    pg_pool: &PgPool,
    source: &OracleFileSourceConfig,
    record: &OracleXmlAuditRecord,
) -> Result<bool, Box<dyn std::error::Error>> {
    let log_time = record.extended_timestamp.unwrap();

    let authentication_type = truncate_opt_string(record.comment_text.clone(), 100);

    let result = sqlx::query(
        r#"
        INSERT INTO oracle_connection_logs (
            log_time,
            action_name,
            dbusername,
            os_username,
            userhost,
            terminal,
            client_program_name,
            authentication_type,
            cluster_name,
            server_name,
            server_ip
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(log_time)
    .bind(&record.action_name)
    .bind(&record.db_user)
    .bind(&record.os_user)
    .bind(&record.userhost)
    .bind(&record.terminal)
    .bind(Option::<String>::None)
    .bind(&authentication_type)
    .bind(&source.cluster_name)
    .bind(&source.server_name)
    .bind(&source.server_ip)
    .execute(pg_pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn insert_audit_log(
    pg_pool: &PgPool,
    source: &OracleFileSourceConfig,
    record: &OracleXmlAuditRecord,
) -> Result<bool, Box<dyn std::error::Error>> {
    let log_time = record.extended_timestamp.unwrap();

    let authentication_type = truncate_opt_string(record.comment_text.clone(), 100);

    let result = sqlx::query(
        r#"
        INSERT INTO oracle_audit_logs (
            log_time,
            action_name,
            dbusername,
            os_username,
            object_schema,
            object_name,
            sql_text,
            userhost,
            terminal,
            client_program_name,
            authentication_type,
            cluster_name,
            server_name,
            server_ip
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(log_time)
    .bind(&record.action_name)
    .bind(&record.db_user)
    .bind(&record.os_user)
    .bind(&record.object_schema)
    .bind(&record.object_name)
    .bind(&record.sql_text)
    .bind(&record.userhost)
    .bind(&record.terminal)
    .bind(Option::<String>::None)
    .bind(&authentication_type)
    .bind(&source.cluster_name)
    .bind(&source.server_name)
    .bind(&source.server_ip)
    .execute(pg_pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

fn build_file_offset_path(
    sincedb_dir: &str,
    source: &OracleFileSourceConfig,
    file_path: &Path,
) -> String {
    let file_key = file_path.to_string_lossy();
    let filename = format!(
        "{}_{}_{}.offset",
        sanitize_filename(&source.cluster_name),
        sanitize_filename(&source.server_name),
        sanitize_filename(&file_key)
    );

    Path::new(sincedb_dir)
        .join(filename)
        .to_string_lossy()
        .to_string()
}

fn read_file_size_offset(path: &str) -> Result<u64, Box<dyn std::error::Error>> {
    if !Path::new(path).exists() {
        return Ok(0);
    }

    let content = fs::read_to_string(path)?;
    let size = content.trim().parse::<u64>().unwrap_or(0);
    Ok(size)
}

fn write_file_size_offset(path: &str, value: u64) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, value.to_string())?;
    Ok(())
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn clean_string(value: &str) -> String {
    value.replace('\0', "").trim().to_string()
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn truncate_opt_string(value: Option<String>, max_len: usize) -> Option<String> {
    value.map(|v| v.chars().take(max_len).collect())
}
