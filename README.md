# Oracle 21c XML Audit File Collector

Oracle 21c XML Audit File Collector reads Oracle 21c XML audit files and exports parsed audit records into PostgreSQL tables.

The project is designed around Oracle Database 21c with `audit_trail = XML, EXTENDED`. Oracle 21c is used because file-based XML extended audit records can be generated and processed reliably.

## Architecture

```text
Oracle 21c Container
        ↓
XML EXTENDED Audit Files
        ↓
Rust Audit File Collector
        ↓
PostgreSQL Target Database
        ├── oracle_connection_logs
        └── oracle_audit_logs
```

The collector reads XML audit files from a shared audit directory. Oracle writes audit files into the host directory `oracle21c-audit/`, and the collector reads the same directory as read-only.

## Repository Layout

```text
oracle21c_xml_audit_collector/
├── README.md
├── docker-compose.yml
├── Dockerfile
├── Cargo.toml
├── config.toml
├── .gitignore
│
├── src/
│   └── main.rs
│
├── offsets/
│   └── .gitkeep
│
├── oracle21c-data/
│   └── .gitkeep
│
└── oracle21c-audit/
    └── .gitkeep
```

## Directory Purpose

| Path | Purpose |
|---|---|
| `docker-compose.yml` | Starts Oracle 21c and the audit collector |
| `Dockerfile` | Builds the Rust collector container |
| `Cargo.toml` | Rust package and dependency configuration |
| `config.toml` | Collector source and runtime configuration |
| `src/main.rs` | Main collector implementation |
| `offsets/` | Stores processed file offsets |
| `oracle21c-data/` | Stores Oracle database files on the host |
| `oracle21c-audit/` | Stores Oracle XML audit files on the host |

Real Oracle data files, XML audit files, and offset files should not be committed to Git.

## Features

- Reads Oracle 21c XML audit files
- Supports `audit_trail = XML, EXTENDED`
- Supports one or more Oracle audit file sources
- Adds source metadata to every record:
  - `cluster_name`
  - `server_name`
  - `server_ip`
- Separates connection events and SQL audit events
- Writes connection events into `oracle_connection_logs`
- Writes SQL audit events into `oracle_audit_logs`
- Uses offset files to avoid reprocessing already-read XML files
- Uses PostgreSQL `ON CONFLICT DO NOTHING` to reduce duplicate inserts
- Filters common Oracle and DBeaver metadata noise
- Runs with Docker Compose

## Requirements

- Docker
- Docker Compose
- PostgreSQL target database
- Git

The PostgreSQL database can run on the host, another container, or a remote server. The collector only needs a valid PostgreSQL connection string.

## Clone the Repository

```bash
git clone <REPOSITORY_URL>
cd <REPOSITORY_DIRECTORY>
```

If this project is inside a larger repository, enter the Oracle collector directory:

```bash
cd oracle21c_xml_audit_collector
```

## Project Directories

The repository already includes the required directories with `.gitkeep` files. If they are missing in your environment, create them with:

```bash
mkdir -p oracle21c-data oracle21c-audit offsets
```

For local test environments, if Oracle cannot write to the bind-mounted directories, adjust permissions:

```bash
chmod -R 777 oracle21c-data oracle21c-audit offsets
```

For production environments, prefer using the correct container user/group ownership instead of broad permissions.

## Configuration

The repository already includes `config.toml`. Update it for your own environment.

```toml
[collector]
poll_interval_secs = 5
sincedb_dir = "/app/offsets"

[[oracle_file_sources]]
cluster_name = "<CLUSTER_NAME>"
server_name = "oracle21c-db"
server_ip = "<ORACLE_SERVER_IP>"
audit_file_path = "/oracle-audit/**/*.xml"
```

### Configuration Fields

| Field | Description |
|---|---|
| `poll_interval_secs` | How often the collector scans XML audit files |
| `sincedb_dir` | Offset directory inside the collector container |
| `cluster_name` | Logical environment or cluster name |
| `server_name` | Oracle source server name |
| `server_ip` | Oracle source server IP address |
| `audit_file_path` | XML audit file glob path inside the collector container |

The default collector container sees Oracle audit files at:

```text
/oracle-audit/**/*.xml
```

This path comes from the Docker Compose volume:

```yaml
- ./oracle21c-audit:/oracle-audit:ro
```

## Docker Compose Configuration

The repository already includes `docker-compose.yml`. Update only environment-specific values such as passwords, ports, and PostgreSQL connection details.

Default structure:

```yaml
services:
  oracle21c-db:
    image: gvenzl/oracle-xe:21-slim
    container_name: oracle21c-db
    restart: unless-stopped
    ports:
      - "1522:1521"
    shm_size: "1g"
    environment:
      ORACLE_PASSWORD: "<ORACLE_SYS_PASSWORD>"
    volumes:
      - ./oracle21c-data:/opt/oracle/oradata
      - ./oracle21c-audit:/opt/oracle/audit

  oracle21c-audit-file-collector:
    build: .
    container_name: oracle21c-audit-file-collector
    restart: unless-stopped
    network_mode: host
    environment:
      POSTGRES_DB_URL: "postgres://<POSTGRES_USER>:<POSTGRES_PASSWORD>@<POSTGRES_HOST>:<POSTGRES_PORT>/<POSTGRES_DATABASE>"
    volumes:
      - ./config.toml:/app/config.toml:ro
      - ./offsets:/app/offsets
      - ./oracle21c-audit:/oracle-audit:ro
```

### Important Volume Mapping

Oracle writes XML audit files to this path inside the Oracle container:

```text
/opt/oracle/audit
```

That path is mounted to the host directory:

```text
./oracle21c-audit
```

The collector reads the same host directory as:

```text
/oracle-audit
```

Therefore, the collector config should use:

```toml
audit_file_path = "/oracle-audit/**/*.xml"
```

## PostgreSQL Target Tables

Create the target tables before starting the collector.

### Connection Logs Table

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

### Audit Logs Table

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

## Recommended Unique Indexes

The collector inserts records with:

```sql
ON CONFLICT DO NOTHING
```

Create unique indexes to make duplicate prevention effective.

### Connection Logs Unique Index

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

### Audit Logs Unique Index

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

## Start the Containers

Build and start the Oracle 21c database and collector:

```bash
docker compose up -d --build
```

Check running containers:

```bash
docker ps
```

Follow Oracle logs:

```bash
docker logs -f oracle21c-db
```

Follow collector logs:

```bash
docker logs -f oracle21c-audit-file-collector
```

## Oracle 21c Audit Setup

Connect to the Oracle container:

```bash
docker exec -it oracle21c-db bash
sqlplus / as sysdba
```

Enable XML extended auditing:

```sql
ALTER SYSTEM SET audit_trail='XML, EXTENDED' SCOPE=SPFILE;
ALTER SYSTEM SET audit_file_dest='/opt/oracle/audit' SCOPE=SPFILE;
```

Restart Oracle after changing `audit_trail`:

```bash
docker restart oracle21c-db
```

Reconnect and verify:

```bash
docker exec -it oracle21c-db bash
sqlplus / as sysdba
```

```sql
SHOW PARAMETER audit_trail;
SHOW PARAMETER audit_file_dest;
```

Expected values:

```text
audit_trail      XML, EXTENDED
audit_file_dest  /opt/oracle/audit
```

## Enable Auditing for an Oracle User

Connect to the target PDB:

```sql
ALTER SESSION SET CONTAINER=<PDB_NAME>;
```

Create a test user if needed:

```sql
CREATE USER <ORACLE_USERNAME> IDENTIFIED BY <ORACLE_PASSWORD>;
GRANT CREATE SESSION TO <ORACLE_USERNAME>;
GRANT CREATE TABLE TO <ORACLE_USERNAME>;
GRANT UNLIMITED TABLESPACE TO <ORACLE_USERNAME>;
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

## Generate Test Audit Records

Connect as the audited user:

```bash
sqlplus <ORACLE_USERNAME>/<ORACLE_PASSWORD>@localhost:1522/<PDB_NAME>
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

## Check XML Audit Files

Check whether XML audit files are generated on the host:

```bash
find oracle21c-audit -type f -name "*.xml" | tail -n 20
```

Check whether SQL text exists in audit files:

```bash
grep -R "Sql_Text\|SELECT\|INSERT\|UPDATE\|DELETE\|CREATE TABLE\|ALTER TABLE\|DROP TABLE" oracle21c-audit | tail -n 100
```

## Check PostgreSQL Output

Audit logs:

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

Connection logs:

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

## Supported Action Mapping

| Oracle Action Code | Action Name |
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

`LOGON` and `LOGOFF` records are written into `oracle_connection_logs`.

All other supported non-filtered actions are written into `oracle_audit_logs`.

## Built-in Filtering

The collector skips common noisy records, including:

- Records without timestamp
- Empty database user records
- `SYS` user records
- Unknown actions
- Oracle system metadata queries
- DBeaver metadata queries
- Queries against `SYS` schema objects

Examples of filtered metadata patterns:

```text
ALL_CONSTRAINTS
ALL_INDEXES
ALL_TAB_COLS
USER_OBJECTS
DBA_POLICIES
SYS.DUAL
```

## Offset Handling

The collector stores processed file sizes under:

```text
offsets/
```

Inside the container, this directory is mounted as:

```text
/app/offsets
```

To reprocess all XML audit files from the beginning:

```bash
docker stop oracle21c-audit-file-collector
rm -f offsets/*.offset
docker start oracle21c-audit-file-collector
```

## Troubleshooting

### XML files are not generated

Check Oracle audit parameters:

```sql
SHOW PARAMETER audit_trail;
SHOW PARAMETER audit_file_dest;
```

Expected:

```text
audit_trail      XML, EXTENDED
audit_file_dest  /opt/oracle/audit
```

If `audit_trail` was changed, restart Oracle:

```bash
docker restart oracle21c-db
```

### SQL text is missing in XML files

Make sure XML extended auditing is enabled:

```sql
ALTER SYSTEM SET audit_trail='XML, EXTENDED' SCOPE=SPFILE;
```

Then restart Oracle.

### Collector cannot connect to PostgreSQL

Check `POSTGRES_DB_URL` in `docker-compose.yml`:

```text
postgres://<POSTGRES_USER>:<POSTGRES_PASSWORD>@<POSTGRES_HOST>:<POSTGRES_PORT>/<POSTGRES_DATABASE>
```

If PostgreSQL runs on the Docker host and the collector uses `network_mode: host`, `localhost` can be used as the PostgreSQL host on Linux.

### Permission denied on Oracle data or audit folders

For local testing:

```bash
chmod -R 777 oracle21c-data oracle21c-audit offsets
```

For production, set the correct ownership for the Oracle container user instead.

### Duplicate records

Check the following:

- Offset files are mounted persistently
- Unique indexes were created in PostgreSQL
- The same audit directory is not configured more than once with the same metadata

## Clean Restart

Stop and remove containers:

```bash
docker compose down
```

Remove collector offsets only:

```bash
rm -f offsets/*.offset
```

Remove Oracle database files and audit files for a full local reset:

```bash
sudo rm -rf oracle21c-data/* oracle21c-audit/* offsets/*
touch oracle21c-data/.gitkeep oracle21c-audit/.gitkeep offsets/.gitkeep
```

Start again:

```bash
docker compose up -d --build
```

## Git Ignore Policy

The repository keeps empty working directories with `.gitkeep`, but ignores generated runtime data.

Tracked:

```text
oracle21c-data/.gitkeep
oracle21c-audit/.gitkeep
offsets/.gitkeep
```

Ignored:

```text
oracle21c-data/*
oracle21c-audit/*
offsets/*
target/
.env
*.log
```

## Security Notes

- Do not commit real passwords.
- Replace placeholder credentials before running.
- Use a restricted PostgreSQL user in production.
- Mount audit files as read-only in the collector container.
- Do not commit Oracle data files, XML audit files, or offset files.

## Notes

- This collector targets Oracle 21c XML extended audit files.
- Oracle 21c was selected as the supported version for this file-based XML collector.
- Oracle 26ai may require a Unified Auditing based collector instead of this XML file-based approach.
