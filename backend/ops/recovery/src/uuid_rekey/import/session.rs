//! One bounded native session per quarantined target. No business row is an SQL
//! literal: COPY data is encoded separately and stdout is consumed one line at a time.
use super::{new_plaintext, strict, Config, Target};
use crate::{native::Scratch, Error, Result};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Session {
    child: Option<Child>,
    input: Option<SyncSender<Vec<u8>>>,
    output: Option<Receiver<Result<Vec<u8>>>>,
    workers: Vec<JoinHandle<Result<()>>>,
    deadline: Instant,
}

pub(super) fn line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut result = Vec::new();
    let length = reader
        .take(super::MAX_LINE + 1)
        .read_until(b'\n', &mut result)
        .map_err(|_| Error("import stream read failed"))?;
    if length == 0 {
        return Ok(None);
    }
    if length as u64 > super::MAX_LINE || result.last() != Some(&b'\n') {
        return Err(Error("import stream framing or line limit invalid"));
    }
    Ok(Some(result))
}

impl Session {
    pub(super) fn start(config: &Config, target: &Target, scratch: &Scratch) -> Result<Self> {
        let stderr = new_plaintext(&scratch.0.join(format!(
            "{}-{}.stderr",
            target.kind,
            uuid::Uuid::now_v7()
        )))?;
        let mut command = Command::new("psql");
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("PG") {
                command.env_remove(name);
            }
        }
        command
            .args([
                "-X", "-q", "-A", "-t", "--no-password", "-v", "ON_ERROR_STOP=1",
                "-v", "FETCH_COUNT=1000", "--host", &config.host, "--port", "5432",
                "--username", "postgres", "--dbname", &target.name, "--file=-",
            ])
            .env("PGPASSFILE", "/dev/null")
            .env("PGCONNECT_TIMEOUT", "10")
            .env("PGOPTIONS", format!("-c default_transaction_read_only=on -c statement_timeout={} -c lock_timeout=3000",config.timeout_seconds*1000))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        Self::start_process(command, Duration::from_secs(config.timeout_seconds))
    }

    fn start_process(mut command: Command, budget: Duration) -> Result<Self> {
        let child = command
            .spawn()
            .map_err(|_| Error("import native process unavailable"))?;
        let mut session = Self {
            child: Some(child),
            input: None,
            output: None,
            workers: Vec::new(),
            deadline: Instant::now() + budget,
        };
        let child = session
            .child
            .as_mut()
            .ok_or(Error("import process unavailable"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(Error("import stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(Error("import stdout unavailable"))?;
        let (input, commands) = mpsc::sync_channel::<Vec<u8>>(1);
        let (records, output) = mpsc::sync_channel(1);
        session.input = Some(input);
        session.output = Some(output);
        session.workers.push(
            thread::Builder::new()
                .name("uuid-import-copy".into())
                .spawn(move || {
                    let mut stdin = stdin;
                    for bytes in commands {
                        stdin
                            .write_all(&bytes)
                            .and_then(|_| stdin.flush())
                            .map_err(|_| Error("import input stream failed"))?;
                    }
                    Ok(())
                })
                .map_err(|_| Error("import writer unavailable"))?,
        );
        session.workers.push(
            thread::Builder::new()
                .name("uuid-import-records".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    loop {
                        match line(&mut reader) {
                            Ok(Some(bytes)) => {
                                if records.send(Ok(bytes)).is_err() {
                                    return Ok(());
                                }
                            }
                            Ok(None) => return Ok(()),
                            Err(error) => {
                                let _ = records.send(Err(error));
                                return Err(Error("import output framing failed"));
                            }
                        }
                    }
                })
                .map_err(|_| Error("import reader unavailable"))?,
        );
        Ok(session)
    }

    pub(super) fn send(&mut self, mut bytes: Vec<u8>) -> Result<()> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(Error("import operation deadline exceeded"));
            }
            match self
                .input
                .as_ref()
                .ok_or(Error("import input already closed"))?
                .try_send(bytes)
            {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    bytes = returned;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(Error("import process closed its input"))
                }
            }
        }
    }

    pub(super) fn sql(&mut self, mut value: String) -> Result<()> {
        // psql --file=- waits for a complete physical input line even when a
        // semicolon already terminates the statement. Keep SQL framing here so
        // every caller, including metadata queries, can receive its response
        // before sending another command. COPY payload uses send separately.
        if !value.ends_with('\n') {
            value.push('\n');
        }
        self.send(value.into_bytes())
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or(Error("import operation deadline exceeded"))?;
            match self
                .output
                .as_ref()
                .ok_or(Error("import output closed"))?
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(bytes) => return bytes.map(Some),
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    pub(super) fn value(&mut self) -> Result<Value> {
        strict(
            &self
                .next()?
                .ok_or(Error("import native evidence ended early"))?,
        )
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        self.input.take();
        if self.next()?.is_some() {
            return Err(Error("unexpected import output after completion"));
        }
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .ok_or(Error("import process missing"))?
                .try_wait()
                .map_err(|_| Error("import process status failed"))?
            {
                if !status.success() {
                    return Err(Error(
                        "import native operation failed; targets remain quarantined",
                    ));
                }
                break;
            }
            if Instant::now() >= self.deadline {
                return Err(Error("import operation deadline exceeded"));
            }
            thread::sleep(Duration::from_millis(5));
        }
        for worker in self.workers.drain(..) {
            worker.join().map_err(|_| Error("import worker failed"))??;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        self.output.take();
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn hierarchy_copy_roundtrip(session: &mut Session, catalog: &Value) {
        use super::super::{csv, sql};
        // Synthetic rows are created only in the disposable PG17 transaction.
        // Roll back their savepoint before importing the same full row bodies.
        session.sql("SET LOCAL search_path=pg_catalog,public;\n\
            INSERT INTO public.tenants(id,name,code) VALUES\
            ('018bcfe5-6800-7000-8000-000000000001','isolated import fixture','isolated_import_fixture');\n\
            SAVEPOINT hierarchy_source;\n\
            INSERT INTO public.organizations(id,tenant_id,parent_id,name) VALUES\
            ('018bcfe5-6800-7000-8000-000000000002','018bcfe5-6800-7000-8000-000000000001',NULL,'root'),\
            ('018bcfe5-6800-7000-8000-000000000003','018bcfe5-6800-7000-8000-000000000001','018bcfe5-6800-7000-8000-000000000002','child');\n\
            INSERT INTO public.departments(id,organization_id,parent_id,name,description) VALUES\
            ('018bcfe5-6800-7000-8000-000000000004','018bcfe5-6800-7000-8000-000000000002',NULL,'root',E'CSV backslash \\\\ quote \" newline\\n'),\
            ('018bcfe5-6800-7000-8000-000000000005','018bcfe5-6800-7000-8000-000000000002','018bcfe5-6800-7000-8000-000000000004','child','isolated');\n\
            SELECT jsonb_build_object(\
            'organizations',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM public.organizations t),\
            'departments',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM public.departments t));\n\
            ROLLBACK TO hierarchy_source;\nRELEASE hierarchy_source;\n\
            SET LOCAL search_path=pg_catalog;".into()).unwrap();
        let source = session.value().unwrap();
        for table in ["organizations", "departments"] {
            let rows = source[table].as_array().unwrap();
            assert_eq!(rows.len(), 2);
            let plan = sql::hierarchy_fixture_plan(catalog, table, rows.len() as u64);
            session.sql(sql::stage_sql(&plan).unwrap()).unwrap();
            for (index, row) in rows.iter().enumerate() {
                session.send(csv(index as u64 + 1, row).unwrap()).unwrap();
            }
            session.sql("\\.\n".into()).unwrap();
            session.sql(sql::insert_verify_sql(&plan).unwrap()).unwrap();
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    session.value().unwrap(),
                    serde_json::json!({
                        "kind":"import_row","schema":"public","table":table,
                        "ordinal":index+1,"row":row
                    })
                );
            }
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({
                    "kind":"table_verified","schema":"public","table":table,"rows":2
                })
            );
            session
                .sql(
                    "SELECT jsonb_build_object('search_path',current_setting('search_path'));"
                        .into(),
                )
                .unwrap();
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({"search_path":"pg_catalog"})
            );
        }
        // Ordinary trigger protection must still reject actual cycles after
        // the importer has restored rows. Catch expected errors server-side.
        session
            .sql(
                "SET LOCAL search_path=pg_catalog,public;\n\
            DO $fixture$ BEGIN\n\
              BEGIN\n\
                UPDATE public.organizations SET parent_id='018bcfe5-6800-7000-8000-000000000003'\
                WHERE id='018bcfe5-6800-7000-8000-000000000002';\n\
                RAISE EXCEPTION 'fixture organization cycle accepted';\n\
              EXCEPTION WHEN raise_exception THEN\n\
                IF SQLERRM<>'organization hierarchy cannot contain a cycle' THEN RAISE; END IF;\n\
              END;\n\
              BEGIN\n\
                UPDATE public.departments SET parent_id='018bcfe5-6800-7000-8000-000000000005'\
                WHERE id='018bcfe5-6800-7000-8000-000000000004';\n\
                RAISE EXCEPTION 'fixture department cycle accepted';\n\
              EXCEPTION WHEN raise_exception THEN\n\
                IF SQLERRM<>'department hierarchy cannot contain a cycle' THEN RAISE; END IF;\n\
              END;\n\
            END $fixture$;\nSET LOCAL search_path=pg_catalog;"
                    .into(),
            )
            .unwrap();
    }

    /// Executed only inside the disposable offline PG17 container built by the
    /// fixture script. A value is required before sending the next statement,
    /// so closing stdin or a later SQL command cannot hide missing LF framing.
    #[test]
    #[ignore = "requires isolated PG17 fixture with actual three-service migrations"]
    fn isolated_pg17_sql_without_newline_and_migration_contracts() {
        use super::super::{catalog_sql, checked_fence, fence_sql, metadata, sql, Target, TARGETS};
        assert_eq!(
            std::env::var("UUID_IMPORT_PG17_FIXTURE").as_deref(),
            Ok("isolated-offline-fixture-v1")
        );
        for (kind, name, owner) in TARGETS {
            let mut command = Command::new("psql");
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("PG") {
                    command.env_remove(key);
                }
            }
            command
                .args([
                    "-X",
                    "-qAt",
                    "--no-password",
                    "-v",
                    "ON_ERROR_STOP=1",
                    "-v",
                    "FETCH_COUNT=1000",
                    "--host",
                    "/var/run/postgresql",
                    "--username",
                    "postgres",
                    "--dbname",
                    name,
                    "--file=-",
                ])
                .env("PGPASSFILE", "/dev/null")
                .env("PGCONNECT_TIMEOUT", "3")
                .env(
                    "PGOPTIONS",
                    "-c default_transaction_read_only=on -c statement_timeout=10000",
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut session = Session::start_process(command, Duration::from_secs(20)).unwrap();
            let probe = "SELECT jsonb_build_object('pg17',current_setting('server_version_num')::int BETWEEN 170000 AND 179999,'fixture',current_setting('cluster_name')='uuid-import-fixture-v1');";
            assert!(!probe.ends_with('\n'));
            session.sql(probe.into()).unwrap();
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({"pg17":true,"fixture":true})
            );
            // The first value arrived while both stdin and this session remain
            // open, before any following statement has been queued.
            session.sql(sql::begin_sql(kind).unwrap()).unwrap();
            let target = Target {
                kind: kind.into(),
                name: name.into(),
                owner: owner.into(),
                expected_schema_sha256: "a".repeat(64),
            };
            session.sql(fence_sql(&target)).unwrap();
            checked_fence(session.value().unwrap(), &target, None).unwrap();
            let metadata_sql = metadata::query(kind).unwrap();
            assert!(!metadata_sql.ends_with('\n'));
            session.sql(metadata_sql).unwrap();
            metadata::validate(kind, &session.value().unwrap()).unwrap();
            session
                .sql(
                    "SELECT jsonb_build_object('search_path',current_setting('search_path'));"
                        .into(),
                )
                .unwrap();
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({"search_path":"pg_catalog"})
            );
            session.sql(catalog_sql(30).unwrap()).unwrap();
            let catalog = session.value().unwrap();
            for key in [
                "relations",
                "uuid_columns",
                "schema_material",
                "constraints",
                "indexes",
                "functions",
                "triggers",
            ] {
                assert!(catalog[key].is_array(), "missing fixture catalog {key}");
            }
            assert!(!catalog["relations"].as_array().unwrap().is_empty());
            if kind == "platform" {
                hierarchy_copy_roundtrip(&mut session, &catalog);
            }
            session.sql(sql::finish_sql(kind).unwrap()).unwrap();
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({"kind":"business_verified"})
            );
            session
                .sql(
                    "SELECT jsonb_build_object('search_path',current_setting('search_path'));"
                        .into(),
                )
                .unwrap();
            assert_eq!(
                session.value().unwrap(),
                serde_json::json!({"search_path":"pg_catalog"})
            );
            session.sql("ROLLBACK;".into()).unwrap();
            session.finish().unwrap();
        }
    }

    #[test]
    fn native_failure_timeout_and_copy_backpressure_remain_bounded() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "exit 7"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut session = Session::start_process(command, Duration::from_secs(2)).unwrap();
        assert!(session.finish().is_err());
        let mut command = Command::new("sh");
        command
            .args(["-c", "exec sleep 2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut session = Session::start_process(command, Duration::from_millis(50)).unwrap();
        for _ in 0..10 {
            if session.send(vec![b'x'; 65536]).is_err() {
                return;
            }
        }
        panic!("blocked native reader did not hit bounded-input deadline");
    }
}
