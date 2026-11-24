use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use super::{case, database_cases, local_cases};

type Result<T> = std::result::Result<T, String>;

#[derive(Default)]
struct Options {
    database: bool,
    seed: Option<u32>,
    clear: bool,
    extend: Option<String>,
    rerun: Option<String>,
    restore: Option<String>,
    journal: bool,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut out = Self::default();
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--database" => out.database = true,
                "--clear" => out.clear = true,
                "--journal" => out.journal = true,
                "--seed" => {
                    out.seed = Some(
                        args.next()
                            .ok_or("missing --seed value")?
                            .parse()
                            .map_err(|_| "--seed requires an unsigned 32-bit integer")?,
                    )
                }
                "--extend" => out.extend = Some(args.next().ok_or("missing --extend test ID")?),
                "--rerun" => out.rerun = Some(args.next().ok_or("missing --rerun test ID")?),
                "--rerun-opt" => {
                    out.restore = Some(args.next().ok_or("missing --rerun-opt value")?)
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }
        if let Some(restore) = out.restore.as_deref() {
            if out.rerun.is_none() || !matches!(restore, "db" | "db-config" | "config" | "none") {
                return Err(
                    "--rerun-opt requires --rerun and one of db, db-config, config or none".into(),
                );
            }
        }
        if out.journal
            && (out.database
                || out.clear
                || out.seed.is_some()
                || out.extend.is_some()
                || out.rerun.is_some())
        {
            return Err("--journal is a standalone argument".into());
        }
        Ok(out)
    }
}

struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct History {
    root: PathBuf,
}

impl History {
    fn open() -> Result<Self> {
        let root = env::var_os("RADIUSCORE_SIM_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("simulation/.state"));
        private_dir(&root)?;
        private_dir(&root.join("journal"))?;
        private_dir(&root.join("db-backup"))?;
        Ok(Self { root })
    }

    fn lock(&self) -> Result<Lock> {
        let path = self.root.join("active-run.lock");
        let _file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| {
                format!("cannot lock simulation history (another run may be active): {err}")
            })?;
        Ok(Lock(path))
    }

    fn runs(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for dir in fs::read_dir(self.root.join("journal")).map_err(|err| err.to_string())? {
            let dir = dir.map_err(|err| err.to_string())?;
            if dir.file_type().map_err(|err| err.to_string())?.is_dir() {
                if let Some(id) = dir.file_name().to_str() {
                    if id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        ids.push(id.to_owned());
                    }
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn find(&self, input: &str) -> Result<(String, Value)> {
        if input.is_empty()
            || input.len() > 16
            || !input.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("test IDs must contain 1–16 hexadecimal digits".into());
        }
        let input = input.to_ascii_lowercase();
        let mut matches = self.runs()?.into_iter().filter(|id| id.starts_with(&input));
        let id = matches
            .next()
            .ok_or_else(|| format!("no journal entry for test ID {input}"))?;
        if matches.next().is_some() {
            return Err(format!("ambiguous test ID {input}; supply more digits"));
        }
        let result = self.read(&id, "result.json")?;
        Ok((id, result))
    }

    fn read(&self, id: &str, name: &str) -> Result<Value> {
        let path = self.root.join("journal").join(id).join(name);
        serde_json::from_slice(
            &fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?,
        )
        .map_err(|err| format!("{}: {err}", path.display()))
    }

    fn journal(&self) -> Result<Value> {
        let mut runs = Vec::new();
        for id in self.runs()? {
            let mut entry = self.read(&id, "result.json").unwrap_or_else(|_| {
                json!({
                    "test_id": id, "status": "interrupted",
                })
            });
            if let Some(object) = entry.as_object_mut() {
                object.remove("tests");
            }
            runs.push(entry);
        }
        runs.sort_by_key(|value| value["started_unix_ms"].as_u64().unwrap_or_default());
        Ok(json!({"status":"passed","runs":runs}))
    }

    fn new_run(&self) -> Result<(String, PathBuf)> {
        loop {
            let mut random = [0; 8];
            openssl::rand::rand_bytes(&mut random).map_err(|err| err.to_string())?;
            let id = format!("{:016x}", u64::from_be_bytes(random));
            let path = self.root.join("journal").join(&id);
            match make_dir(&path) {
                Ok(()) => return Ok((id, path)),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.to_string()),
            }
        }
    }

    fn backups(&self, id: &str) -> PathBuf {
        self.root.join("db-backup").join(id)
    }
}

fn make_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn private_dir(path: &Path) -> Result<()> {
    match make_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(err) => Err(format!("{}: {err}", path.display())),
    }
}

fn save(path: &Path, value: &Value) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp).map_err(|err| err.to_string())?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|err| err.to_string())?;
    writeln!(file).map_err(|err| err.to_string())?;
    file.sync_all().map_err(|err| err.to_string())?;
    fs::rename(tmp, path).map_err(|err| err.to_string())
}

fn pg(mut command: Command, label: &str) -> Result<Vec<u8>> {
    let output = command
        .env("PGCONNECT_TIMEOUT", "5")
        .output()
        .map_err(|err| format!("{label}: {err}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    eprintln!(
        "{label}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Err(format!("{label} failed; see stderr"))
}

fn database_target() -> Result<Value> {
    let name = env::var("PGDATABASE")
        .map_err(|_| "set PGDATABASE to a disposable radiuscore_test database")?;
    if !(name == "radiuscore_test"
        || name.starts_with("radiuscore_test_")
        || name == "radiuscore_ci")
        || name.len() > 63
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    {
        return Err(
            "database snapshots and resets require PGDATABASE=radiuscore_test or radiuscore_test_*"
                .into(),
        );
    }
    let mut command = Command::new("psql");
    command.args([
        "-XqAt",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        concat!(
            "SELECT json_build_object('name',current_database(),",
            "'server',inet_server_addr()::text,'port',current_setting('port'),",
            "'extra_schemas',(SELECT count(*) FROM pg_namespace WHERE nspname NOT IN ",
            "('public','pg_catalog','information_schema') AND nspname !~ '^pg_(toast|temp)'))"
        ),
    ]);
    let result: Value = serde_json::from_slice(&pg(command, "database validation")?)
        .map_err(|err| format!("invalid database identity: {err}"))?;
    if result["name"] != name || result["extra_schemas"] != 0 {
        return Err("the connected database must be the selected test database and contain only the public application schema".into());
    }
    Ok(result)
}

fn dump(path: &Path) -> Result<()> {
    let temp = path.with_extension("partial");
    let mut command = Command::new("pg_dump");
    command
        .args(["--format=custom", "--lock-wait-timeout=5s", "--file"])
        .arg(&temp);
    pg(command, "database backup")?;
    File::open(&temp)
        .and_then(|file| file.sync_all())
        .map_err(|err| err.to_string())?;
    fs::rename(temp, path).map_err(|err| err.to_string())
}

fn apply_sql(paths: &[PathBuf]) -> Result<()> {
    let mut command = Command::new("psql");
    command
        .args(["-Xq", "--single-transaction", "-v", "ON_ERROR_STOP=1"])
        .env("PGOPTIONS", "-c lock_timeout=5000");
    for path in paths {
        command.arg("-f").arg(path);
    }
    pg(command, "database reset or restore").map(|_| ())
}

fn reset(folder: &Path, archive: Option<&Path>) -> Result<()> {
    let prelude = folder.join("reset.sql");
    let mut files = vec![prelude.clone()];
    let mut create_public = true;
    if let Some(archive) = archive {
        let mut list = Command::new("pg_restore");
        list.arg("--list").arg(archive);
        let toc = pg(list, "snapshot validation")?;
        create_public = !String::from_utf8_lossy(&toc).lines().any(|line| {
            line.split_whitespace()
                .collect::<Vec<_>>()
                .windows(3)
                .any(|entry| entry == ["SCHEMA", "-", "public"])
        });
        let snapshot = folder.join("restore.sql");
        let mut restore = Command::new("pg_restore");
        restore
            .args(["--no-owner", "--no-privileges", "--file"])
            .arg(&snapshot)
            .arg(archive);
        pg(restore, "snapshot extraction")?;
        files.push(snapshot);
    } else {
        let migration_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut migrations: Vec<_> = fs::read_dir(migration_dir)
            .map_err(|err| err.to_string())?
            .collect::<io::Result<Vec<_>>>()
            .map_err(|err| err.to_string())?
            .into_iter()
            .map(|f| f.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
            .collect();
        migrations.sort();
        files.extend(migrations);
    }
    let sql = if create_public {
        "DROP SCHEMA IF EXISTS public CASCADE;\nCREATE SCHEMA public AUTHORIZATION pg_database_owner;\n"
    } else {
        "DROP SCHEMA IF EXISTS public CASCADE;\n"
    };
    fs::write(&prelude, sql).map_err(|err| err.to_string())?;
    let result = apply_sql(&files);
    let _ = fs::remove_file(prelude);
    let _ = fs::remove_file(folder.join("restore.sql"));
    result
}

fn announce(notices: &mut Vec<String>, message: &str) {
    eprintln!("{message}");
    notices.push(message.to_owned());
}

pub(super) fn execute() -> Result<ExitCode> {
    let args = Options::parse()?;
    let store = History::open()?;
    let _lock = store.lock()?;
    if args.journal {
        println!(
            "{}",
            serde_json::to_string_pretty(&store.journal()?).unwrap()
        );
        return Ok(ExitCode::SUCCESS);
    }
    let extension = args
        .extend
        .as_deref()
        .map(|id| store.find(id))
        .transpose()?;
    let rerun = args.rerun.as_deref().map(|id| store.find(id)).transpose()?;
    let restore = args.restore.as_deref().unwrap_or("none");
    let restore_database = matches!(restore, "db" | "db-config");
    let restore_config = matches!(restore, "config" | "db-config");
    let mut notices = Vec::new();
    if extension.is_some() && rerun.is_some() {
        announce(&mut notices, "Both --extend and --rerun were supplied. Default selection is extend; --rerun supplies the requested snapshots.");
    }
    let history = extension.as_ref().or(rerun.as_ref());
    let restored_config = if restore_config {
        Some(store.read(
            &rerun.as_ref().expect("validated options").0,
            "settings.json",
        )?)
    } else {
        None
    };
    let previous = history.map(|(_, run)| &run["settings"]);
    let configured = restored_config
        .as_ref()
        .or_else(|| extension.as_ref().map(|(_, run)| &run["settings"]));
    let seed = args
        .seed
        .or_else(|| {
            configured
                .and_then(|v| v["seed"].as_u64())
                .map(|v| v as u32)
        })
        .unwrap_or(27);
    let database = args.database
        || restore_database
        || previous.is_some_and(|v| v["database"] == true)
        || restored_config
            .as_ref()
            .is_some_and(|v| v["database"] == true);
    if args.clear && !database {
        return Err("--clear requires a database run".into());
    }
    if args.seed.is_some() && restore_config {
        announce(
            &mut notices,
            "Explicit --seed overrides the restored configuration seed.",
        );
    }
    if args.clear && restore_database {
        announce(&mut notices, "Database restore takes precedence over resetting to empty migrations; the current database is backed up first.");
    }

    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_millis() as u64;
    let (id, folder) = store.new_run()?;
    let settings = json!({"seed":seed,"database":database});
    save(&folder.join("settings.json"), &settings)?;
    let mut tests = Vec::new();
    local_cases(&mut tests);
    let mut target = Value::Null;
    let mut before = false;
    let mut input = false;
    if database {
        let prepare = (|| -> Result<()> {
            target = database_target()?;
            let backups = store.backups(&id);
            private_dir(&backups)?;
            let source = if restore_database {
                let (old_id, old_run) = rerun.as_ref().expect("validated options");
                let snapshot = store.backups(old_id).join("input.dump");
                if !snapshot.is_file() {
                    return Err("selected rerun has no input database backup".into());
                }
                if old_run["database_target"] != target {
                    return Err(
                        "snapshot database identity differs from the connected test database"
                            .into(),
                    );
                }
                Some(snapshot)
            } else {
                None
            };
            if args.clear || restore_database {
                dump(&backups.join("before-change.dump"))?;
                before = true;
                reset(&folder, source.as_deref())?;
            }
            dump(&backups.join("input.dump"))?;
            input = true;
            database_cases(seed, &mut tests)
        })();
        if let Err(error) = prepare {
            case(
                &mut tests,
                "postgresql_execution",
                json!("completed"),
                json!(error),
            );
        }
    }
    let failed = tests
        .iter()
        .filter(|test| test["status"] == "failed")
        .count();
    let status = if failed != 0 {
        "failed"
    } else if database {
        "passed"
    } else {
        "partial"
    };
    let report = json!({
        "status":status, "test_id":id, "started_unix_ms":started, "seed":seed, "settings":settings,
        "selection":if extension.is_some() {"extend"} else if rerun.is_some() {"rerun"} else {"current"},
        "extends":extension.as_ref().map(|(id,_)|id), "rerun_of":rerun.as_ref().map(|(id,_)|id),
        "rerun_opt":restore, "notices":notices, "database_target":target,
        "snapshots":{"before_change":before.then(||format!("db-backup/{id}/before-change.dump")),
            "input":input.then(||format!("db-backup/{id}/input.dump")),
            "settings":format!("journal/{id}/settings.json")},
        "scope":"in-process packet, admission, and optional PostgreSQL simulation",
        "summary":{"passed":tests.len()-failed,"failed":failed,"skipped_groups":if database {vec![]} else {vec!["postgresql"]}},
        "tests":tests,
    });
    save(&folder.join("result.json"), &report)?;
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
