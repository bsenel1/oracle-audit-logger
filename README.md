# Oracle 21c XML Audit File Collector

This project collects Oracle 21c XML audit files, parses Oracle audit records, separates connection events from SQL audit events, and writes the parsed data into PostgreSQL tables.

Oracle 21c is the target version for this collector. The project is designed for Oracle 21c with `audit_trail = XML, EXTENDED` because this mode produces file-based XML audit records that can be collected without connecting directly to the Oracle database from the collector.

## Architecture

```text
Oracle 21c
  -> XML EXTENDED audit files
  -> Oracle XML Audit File Collector
  -> PostgreSQL
       -> oracle_connection_logs
       -> oracle_audit_logs
```

## Features

- Reads Oracle 21c XML audit files.
- Supports `audit_trail = XML, EXTENDED`.
- Supports one or more Oracle audit file sources.
- Adds source metadata to every record:
  - `cluster_name`
  - `server_name`
  - `server_ip`
- Separates connection events and SQL audit events.
- Writes connection events to `oracle_connection_logs`.
- Writes SQL audit events to `oracle_audit_logs`.
- Stores file offsets under the configured `sincedb_dir`.
- Prevents duplicate inserts with `ON CONFLICT DO NOTHING` when the required unique indexes exist.
- Filters common Oracle and DBeaver metadata queries.
- Supports Docker-based deployment.

## Repository Structure

```text
.
├── docker-compose.yml
├── oracle21c-audit/
└── oracle_audit_file_collector/
    ├── Cargo.toml
    ├── Dockerfile
    ├── config.toml
    ├── offsets/
    └── src/
        └── main.rs
```

The repository already contains the required files. Users should not create these files from scratch. They only need to edit the placeholder values in the existing files.

## Required Components

- Docker
- Docker Compose
- PostgreSQL target database
- Oracle Database 21c XE
- Rust only if running the collector without Docker

## Configuration Files

The main files that usually need environment-specific changes are:

```text
config.toml
docker-compose.yml
```

Use placeholder values such as `<POSTGRES_USER>` and `<ORACLE_SYS_PASSWORD>` in committed examples. Do not commit real credentials, IP addresses, or passwords.

## config.toml

The repository includes `oracle_audit_file_collector/config.toml`.

Default template:

```toml
[collector]
poll_interval_secs = 5
sincedb_dir = "/app/offsets"

[[oracle_file_sources]]
cluster_name = "<CLUSTER_NAME>"
server_name = "<ORACLE_SERVER_NAME>"
server_ip = "<ORACLE_SERVER_IP>"
audit_file_path = "/oracle-audit/**/*.xml"
```

### Configuration Fields

| Field | Description |
|---|---|
| `poll_interval_secs` | How often the collector scans XML audit files. |
| `sincedb_dir` | Directory where offset files are stored inside the collector container. |
| `cluster_name` | Logical cluster or environment name. |
| `server_name` | Oracle source server name. |
| `server_ip` | Oracle source server IP address. |
| `audit_file_path` | Glob pattern for XML audit files as seen from inside the collector container. |

Example source configuration:

```toml
[[oracle_file_sources]]
cluster_name = "<PRODUCTION_CLUSTER>"
server_name = "<ORACLE_SERVER_01>"
server_ip = "<ORACLE_SERVER_IP>"
audit_file_path = "/oracle-audit/**/*.xml"
```

For multiple Oracle sources, add another `[[oracle_file_sources]]` block:

```toml
[[oracle_file_sources]]
cluster_name = "<CLUSTER_NAME_1>"
server_name = "<ORACLE_SERVER_NAME_1>"
server_ip = "<ORACLE_SERVER_IP_1>"
audit_file_path = "/oracle-audit/server-1/**/*.xml"

[[oracle_file_sources]]
cluster_name = "<CLUSTER_NAME_2>"
server_name = "<ORACLE_SERVER_NAME_2>"
server_ip = "<ORACLE_SERVER_IP_2>"
audit_file_path = "/oracle-audit/server-2/**/*.xml"
```

## docker-compose.yml

The repository includes `docker-compose.yml` at the repository root.

Default template:

```yaml
services:
  oracle21c-db:
    image: gvenzl/oracle-xe:21-slim
    container_name: oracle21c-db
    restart: unless-stopped
    ports:
      - "<HOST_ORACLE_PORT>:1521"
    shm_size: "1g"
    environment:
      ORACLE_PASSWORD: "<ORACLE_SYS_PASSWORD>"
    volumes:
      - oracle21c_data:/opt/oracle/oradata
      - ./oracle21c-audit:<ORACLE_AUDIT_DIRECTORY_IN_ORACLE_CONTAINER>

  oracle21c-audit-file-collector:
    build: ./oracle_audit_file_collector
    container_name: oracle21c-audit-logger
    restart: unless-stopped
    network_mode: host
    environment:
      POSTGRES_DB_URL: "postgres://<POSTGRES_USER>:<POSTGRES_PASSWORD>@<POSTGRES_HOST>:<POSTGRES_PORT>/<POSTGRES_DATABASE>"
    volumes:
      - ./oracle_audit_file_collector/config.toml:/app/config.toml:ro
      - ./oracle_audit_file_collector/offsets:/app/offsets
      - ./oracle21c-audit:/oracle-audit:ro

volumes:
  oracle21c_data:
```

### Important Volume Mapping

Oracle writes XML audit files into this container path:

```text
<ORACLE_AUDIT_DIRECTORY_IN_ORACLE_CONTAINER>
```

The host stores those files in:

```text
./oracle21c-audit
```

The collector reads the same host directory through this path:

```text
/oracle-audit
```

Therefore, the default `config.toml` audit pattern should usually remain:

```toml
audit_file_path = "/oracle-audit/**/*.xml"
```

A common Oracle audit directory example is:

```text
/opt/oracle/audit
```

In that case, the Compose volume can be:

```yaml
- ./oracle21c-audit:/opt/oracle/audit
```

and the collector volume remains:

```yaml
- ./oracle21c-audit:/oracle-audit:ro
```

## PostgreSQL Target Tables

Create the PostgreSQL target tables before starting the collector.

### oracle_connection_logs

```sql
CREATE TABLE IF NOT EXISTS oracle_connection_logs (
    id BIGSERIAL PRIMARY KEY,
    log_time TIMESTAMPTZ NOT NULL,
    action_name TEXT NOT NULL,
    dbusername TEXT,
    os_username TEXT,
    userhost TEXT,
    terminal TEXT,
    client_program_name TEXT,
    authentication_type TEXT,
    cluster_name TEXT,
    server_name TEXT,
    server_ip TEXT
);
```

### oracle_audit_logs

```sql
CREATE TABLE IF NOT EXISTS oracle_audit_logs (
    id BIGSERIAL PRIMARY KEY,
    log_time TIMESTAMPTZ NOT NULL,
    action_name TEXT NOT NULL,
    dbusername TEXT,
    os_username TEXT,
    object_schema TEXT,
    object_name TEXT,
    sql_text TEXT,
    userhost TEXT,
    terminal TEXT,
    client_program_name TEXT,
    authentication_type TEXT,
    cluster_name TEXT,
    server_name TEXT,
    server_ip TEXT
);
```

## Recommended Indexes

```sql
CREATE INDEX IF NOT EXISTS idx_oracle_connection_log_time
ON oracle_connection_logs (log_time DESC);

CREATE INDEX IF NOT EXISTS idx_oracle_audit_log_time
ON oracle_audit_logs (log_time DESC);

CREATE INDEX IF NOT EXISTS idx_oracle_connection_server_time
ON oracle_connection_logs (server_name, log_time DESC);

CREATE INDEX IF NOT EXISTS idx_oracle_audit_server_time
ON oracle_audit_logs (server_name, log_time DESC);

CREATE INDEX IF NOT EXISTS idx_oracle_audit_cluster_time
ON oracle_audit_logs (cluster_name, log_time DESC);
```

## Unique Indexes for Duplicate Prevention

The collector uses `ON CONFLICT DO NOTHING`. To make this effective, create unique indexes.

### oracle_connection_logs Unique Index

```sql
CREATE UNIQUE INDEX IF NOT EXISTS uq_oracle_connection_logs_dedup
ON oracle_connection_logs (
    log_time,
    action_name,
    COALESCE(dbusername, ''),
    COALESCE(os_username, ''),
    COALESCE(userhost, ''),
    COALESCE(terminal, ''),
    COALESCE(authentication_type, ''),
    COALESCE(cluster_name, ''),
    COALESCE(server_name, ''),
    COALESCE(server_ip, '')
);
```

### oracle_audit_logs Unique Index

```sql
CREATE UNIQUE INDEX IF NOT EXISTS uq_oracle_audit_logs_dedup
ON oracle_audit_logs (
    log_time,
    action_name,
    COALESCE(dbusername, ''),
    COALESCE(os_username, ''),
    COALESCE(object_schema, ''),
    COALESCE(object_name, ''),
    COALESCE(sql_text, ''),
    COALESCE(userhost, ''),
    COALESCE(terminal, ''),
    COALESCE(authentication_type, ''),
    COALESCE(cluster_name, ''),
    COALESCE(server_name, ''),
    COALESCE(server_ip, '')
);
```

## Oracle 21c Audit Configuration

Connect to Oracle as SYSDBA:

```bash
docker exec -it oracle21c-db bash
sqlplus / as sysdba
```

Set XML extended auditing:

```sql
ALTER SYSTEM SET audit_trail='XML, EXTENDED' SCOPE=SPFILE;
ALTER SYSTEM SET audit_file_dest='<ORACLE_AUDIT_DIRECTORY_IN_ORACLE_CONTAINER>' SCOPE=SPFILE;
```

Example:

```sql
ALTER SYSTEM SET audit_trail='XML, EXTENDED' SCOPE=SPFILE;
ALTER SYSTEM SET audit_file_dest='/opt/oracle/audit' SCOPE=SPFILE;
```

Restart Oracle after changing `audit_trail`:

```bash
docker restart oracle21c-db
```

Verify the settings:

```sql
SHOW PARAMETER audit_trail;
SHOW PARAMETER audit_file_dest;
```

Expected result:

```text
audit_trail      XML, EXTENDED
audit_file_dest  <ORACLE_AUDIT_DIRECTORY_IN_ORACLE_CONTAINER>
```

## Enable Oracle Auditing for a User

Switch to the target PDB:

```sql
ALTER SESSION SET CONTAINER=<PDB_NAME>;
```

Enable session auditing:

```sql
AUDIT SESSION BY <ORACLE_USERNAME> BY ACCESS;
```

Enable DML auditing:

```sql
AUDIT SELECT TABLE BY <ORACLE_USERNAME> BY ACCESS;
AUDIT INSERT TABLE BY <ORACLE_USERNAME> BY ACCESS;
AUDIT UPDATE TABLE BY <ORACLE_USERNAME> BY ACCESS;
AUDIT DELETE TABLE BY <ORACLE_USERNAME> BY ACCESS;
```

Enable DDL auditing:

```sql
AUDIT CREATE TABLE BY <ORACLE_USERNAME> BY ACCESS;
AUDIT ALTER TABLE BY <ORACLE_USERNAME> BY ACCESS;
AUDIT DROP TABLE BY <ORACLE_USERNAME> BY ACCESS;
```

Example with placeholders:

```sql
ALTER SESSION SET CONTAINER=<PDB_NAME>;

AUDIT SESSION BY <APP_USER> BY ACCESS;

AUDIT SELECT TABLE BY <APP_USER> BY ACCESS;
AUDIT INSERT TABLE BY <APP_USER> BY ACCESS;
AUDIT UPDATE TABLE BY <APP_USER> BY ACCESS;
AUDIT DELETE TABLE BY <APP_USER> BY ACCESS;

AUDIT CREATE TABLE BY <APP_USER> BY ACCESS;
AUDIT ALTER TABLE BY <APP_USER> BY ACCESS;
AUDIT DROP TABLE BY <APP_USER> BY ACCESS;
```

## Running the Project

After updating the existing `config.toml` and `docker-compose.yml` files, start the project from the repository root:

```bash
docker compose up -d --build
```

Check containers:

```bash
docker ps
```

Follow collector logs:

```bash
docker logs -f oracle21c-audit-logger
```

Expected collector output:

```text
Oracle 21c XML audit logger started.
[<CLUSTER_NAME> / <ORACLE_SERVER_NAME>] Oracle XML audit: <N> new audit records, <N> new connection records
```

## Running the Collector Manually

If Oracle and PostgreSQL are already running, the collector can be built and run manually.

Build:

```bash
cd oracle_audit_file_collector
docker build -t oracle-audit-file-collector:latest .
```

Run:

```bash
docker run -d \
  --name oracle21c-audit-logger \
  --network host \
  -e POSTGRES_DB_URL="postgres://<POSTGRES_USER>:<POSTGRES_PASSWORD>@<POSTGRES_HOST>:<POSTGRES_PORT>/<POSTGRES_DATABASE>" \
  -v "$(pwd)/config.toml:/app/config.toml:ro" \
  -v "$(pwd)/offsets:/app/offsets" \
  -v "<HOST_ORACLE_AUDIT_DIRECTORY>:/oracle-audit:ro" \
  oracle-audit-file-collector:latest
```

## Offset Files

The collector stores file offsets in the directory configured by `sincedb_dir`.

Default value:

```toml
sincedb_dir = "/app/offsets"
```

This directory is mounted from the host:

```yaml
- ./oracle_audit_file_collector/offsets:/app/offsets
```

To reprocess all XML files, stop the collector and delete the offset files:

```bash
docker rm -f oracle21c-audit-logger
rm -f oracle_audit_file_collector/offsets/*.offset
```

Then start the collector again.

## Supported Oracle Action Codes

The collector maps Oracle XML audit action codes to action names.

| Action Code | Action Name |
|---:|---|
| `100` | `LOGON` |
| `101` | `LOGOFF` |
| `1` | `CREATE TABLE` |
| `2` | `INSERT` |
| `3` | `SELECT` |
| `6` | `UPDATE` |
| `7` | `DELETE` |
| `12` | `DROP TABLE` |
| `15` | `ALTER TABLE` |

`LOGON` and `LOGOFF` are inserted into `oracle_connection_logs`.

Other non-filtered actions are inserted into `oracle_audit_logs`.

## Built-in Filtering

The collector skips noisy or unnecessary records such as:

- Records without timestamp
- Empty database users
- `SYS` user records
- Unknown action records
- Oracle system schema records
- Common Oracle metadata queries
- Common DBeaver metadata queries

Examples of filtered patterns:

```text
ALL_CONSTRAINTS
ALL_CONS_COLUMNS
ALL_INDEXES
ALL_IND_COLUMNS
ALL_TAB_COLS
ALL_TABLES
USER_OBJECTS
DBA_POLICIES
XS_SYS_CONTEXT
SYS.DUAL
```

## Test Queries

Connect as the audited Oracle user:

```bash
sqlplus <ORACLE_USERNAME>/<ORACLE_PASSWORD>@<ORACLE_HOST>:<ORACLE_PORT>/<PDB_NAME>
```

Run test SQL:

```sql
CREATE TABLE <TEST_TABLE_NAME> (
    id NUMBER,
    name VARCHAR2(100)
);

INSERT INTO <TEST_TABLE_NAME> VALUES (1, 'test');

SELECT * FROM <TEST_TABLE_NAME>;

UPDATE <TEST_TABLE_NAME>
SET name = 'updated'
WHERE id = 1;

DELETE FROM <TEST_TABLE_NAME>
WHERE id = 1;

ALTER TABLE <TEST_TABLE_NAME>
ADD description VARCHAR2(200);

DROP TABLE <TEST_TABLE_NAME>;
```

Check audit logs in PostgreSQL:

```sql
SELECT
    log_time,
    action_name,
    dbusername,
    object_schema,
    object_name,
    sql_text,
    cluster_name,
    server_name,
    server_ip
FROM oracle_audit_logs
ORDER BY log_time DESC
LIMIT 20;
```

Check connection logs in PostgreSQL:

```sql
SELECT
    log_time,
    action_name,
    dbusername,
    os_username,
    userhost,
    authentication_type,
    cluster_name,
    server_name,
    server_ip
FROM oracle_connection_logs
ORDER BY log_time DESC
LIMIT 20;
```

## Checking XML Audit Files

Check whether Oracle XML audit files are being created:

```bash
find <HOST_ORACLE_AUDIT_DIRECTORY> -type f -name "*.xml" | tail -n 20
```

Search for SQL text inside audit files:

```bash
grep -R "Sql_Text\|SELECT\|INSERT\|UPDATE\|DELETE\|CREATE TABLE\|ALTER TABLE\|DROP TABLE" <HOST_ORACLE_AUDIT_DIRECTORY> | tail -n 100
```

## Troubleshooting

### XML audit files are not created

Check Oracle parameters:

```sql
SHOW PARAMETER audit_trail;
SHOW PARAMETER audit_file_dest;
```

Expected values:

```text
audit_trail      XML, EXTENDED
audit_file_dest  <ORACLE_AUDIT_DIRECTORY_IN_ORACLE_CONTAINER>
```

If `audit_trail` was changed with `SCOPE=SPFILE`, restart Oracle.

### SQL text is missing from XML files

Make sure auditing is configured as XML extended:

```sql
ALTER SYSTEM SET audit_trail='XML, EXTENDED' SCOPE=SPFILE;
```

Then restart Oracle.

### Collector cannot connect to PostgreSQL

Check `POSTGRES_DB_URL`:

```bash
echo $POSTGRES_DB_URL
```

Expected format:

```text
postgres://<POSTGRES_USER>:<POSTGRES_PASSWORD>@<POSTGRES_HOST>:<POSTGRES_PORT>/<POSTGRES_DATABASE>
```

If the collector runs with `network_mode: host`, `localhost` points to the host network namespace.

### Permission denied while reading audit files

Make sure the Oracle audit directory is mounted into the collector container as read-only:

```yaml
- ./oracle21c-audit:/oracle-audit:ro
```

Also make sure the files are readable on the host.

### Duplicate records are inserted

Check the following:

- Unique indexes exist in PostgreSQL.
- Offset files are mounted persistently.
- The same audit path is not configured more than once with the same source metadata.
- Offset files were not deleted while the collector was running.

## Security Notes

- Do not commit real passwords.
- Do not commit real IP addresses if the repository is public.
- Keep `POSTGRES_DB_URL` values environment-specific.
- Mount audit files as read-only in the collector container.
- Use a PostgreSQL user with only the required table permissions in production.

## Notes About Oracle Versions

This collector targets Oracle 21c XML extended auditing.

Oracle 26ai may behave differently because unified auditing is the preferred audit architecture in newer Oracle versions. For this reason, Oracle 21c was selected for this file-based XML audit collector.
