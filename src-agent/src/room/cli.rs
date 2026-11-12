//! `koma room …`: views of a secure-room session.
//!
//! `koma room trajectory` draws the room's map from the shelf. It opens the
//! session's database read-only, so it is safe to run against a live session:
//! it creates no tables and changes no data, schema or journal mode, and the
//! database file stays byte for byte the same. SQLite may create its `-wal` and
//! `-shm` index files beside a WAL database, which it needs in order to read it
//! (a live session already has them).
//!
//! `koma room attach` talks to a running secure daemon. It does not open the
//! database.

use std::path::{Path, PathBuf};

use super::shelf::Shelf;
use super::trajectory::{self, Format};

const USAGE: &str = "usage: koma room trajectory [--goal KEY] [--format ascii|json|dot|mermaid]
                              [--view outline|layered] [--width COLS] [--session ID]
       koma room attach --session ID

  trajectory draws the map from the shelf. The default is an outline tree;
  --view layered draws columns by link depth. Without --session the newest
  room session for the current directory is used.
  attach prints a running secure daemon and sends each typed line as the user.";

/// What `koma room …` was asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomCli {
    Trajectory(TrajectoryArgs),
    /// Live client for one secure daemon.
    Attach { session: String },
    /// Not understood; the text says why.
    Usage(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrajectoryArgs {
    pub goal: Option<String>,
    pub format: Format,
    pub layered: bool,
    pub width: Option<usize>,
    pub session: Option<String>,
}

impl RoomCli {
    /// Parse the arguments after the `room` verb.
    pub fn parse(args: &[String]) -> RoomCli {
        match args.first().map(String::as_str) {
            Some("trajectory") => match parse_trajectory(&args[1..]) {
                Ok(parsed) => RoomCli::Trajectory(parsed),
                Err(why) => RoomCli::Usage(why),
            },
            Some("attach") => match parse_attach(&args[1..]) {
                Ok(session) => RoomCli::Attach { session },
                Err(why) => RoomCli::Usage(why),
            },
            Some(other) => RoomCli::Usage(format!("unknown room command '{other}'")),
            None => RoomCli::Usage("missing room command".to_string()),
        }
    }
}

fn parse_trajectory(args: &[String]) -> Result<TrajectoryArgs, String> {
    let mut out = TrajectoryArgs { goal: None, format: Format::Ascii, layered: false, width: None, session: None };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--goal" => out.goal = Some(value("--goal")?),
            "--session" => out.session = Some(value("--session")?),
            "--format" => {
                let v = value("--format")?;
                out.format = Format::parse(&v).ok_or_else(|| format!("unknown format '{v}' (ascii, json, dot, mermaid)"))?;
            }
            "--view" => match value("--view")?.as_str() {
                "outline" => out.layered = false,
                "layered" => out.layered = true,
                v => return Err(format!("unknown view '{v}' (outline, layered)")),
            },
            "--width" => {
                let v = value("--width")?;
                out.width = Some(v.parse::<usize>().ok().filter(|w| *w >= 20).ok_or_else(|| format!("--width must be a number of at least 20, got '{v}'"))?);
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(out)
}

fn parse_attach(args: &[String]) -> Result<String, String> {
    let mut session = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--session" => {
                session = Some(it.next().cloned().ok_or_else(|| "--session needs a value".to_string())?);
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    let Some(session) = session else {
        return Err("attach needs --session <id>".to_string());
    };
    if session.contains(['/', '\\']) || session == ".." || session == "." {
        return Err(format!("'{session}' is not a session id"));
    }
    Ok(session)
}

/// A session directory is a room session when it holds a room database.
fn has_room(dir: &Path) -> bool {
    super::room_db_path(dir).is_file()
}

/// Pick the session to draw. With an id: that session, looked up in `bucket`
/// first and then anywhere under `root`. Without: the newest room session in
/// `bucket` (the current directory's sessions).
pub fn resolve_session(root: &Path, bucket: &Path, session: Option<&str>) -> Result<PathBuf, String> {
    if let Some(id) = session {
        if id.contains(['/', '\\']) || id == ".." || id == "." {
            return Err(format!("'{id}' is not a session id"));
        }
        let mut candidates = vec![bucket.join(id)];
        if let Ok(buckets) = std::fs::read_dir(root) {
            candidates.extend(buckets.flatten().map(|b| b.path().join(id)));
        }
        return candidates
            .into_iter()
            .find(|dir| has_room(dir))
            .ok_or_else(|| format!("no room session '{id}' found"));
    }
    let mut rooms: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(bucket)
        .map_err(|_| format!("no sessions for this directory under {}", bucket.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|dir| has_room(dir))
        .filter_map(|dir| Some((std::fs::metadata(super::room_db_path(&dir)).ok()?.modified().ok()?, dir)))
        .collect();
    rooms.sort();
    rooms
        .pop()
        .map(|(_, dir)| dir)
        .ok_or_else(|| "no secure-room session for this directory yet (start one with /security)".to_string())
}

/// Draw the trajectory of the room database at `db`.
pub fn render_trajectory(db: &Path, args: &TrajectoryArgs, default_width: usize) -> Result<String, String> {
    let shelf = Shelf::open_read_only(db).map_err(|e| e.0)?;
    let views = trajectory::project(&shelf, args.goal.as_deref())?;
    if views.is_empty() {
        return Err(match &args.goal {
            Some(goal) => format!("no goal '{goal}' on the shelf"),
            None => "nothing on the shelf yet".to_string(),
        });
    }
    let width = args.width.unwrap_or(default_width);
    let text = trajectory::render_all(&views, args.format, args.layered.then_some(width))?;
    Ok(if text.ends_with('\n') { text } else { format!("{text}\n") })
}

fn run_trajectory(args: &TrajectoryArgs) -> Result<String, String> {
    use crate::model::store;
    let root = store::sessions_dir().map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let bucket = store::pwd_bucket_dir(&store::pwd_hash(&cwd)).map_err(|e| e.to_string())?;
    let session = resolve_session(&root, &bucket, args.session.as_deref())?;
    let columns = std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()).filter(|c| *c >= 20).unwrap_or(100);
    render_trajectory(&super::room_db_path(&session), args, columns)
}

/// Run a `koma room` command; the process exit code.
pub fn run(cli: RoomCli) -> i32 {
    match cli {
        RoomCli::Trajectory(args) => match run_trajectory(&args) {
            Ok(text) => {
                print!("{text}");
                0
            }
            Err(why) => {
                eprintln!("koma room trajectory: {why}");
                1
            }
        },
        RoomCli::Attach { session } => match attach::run(&session) {
            Ok(()) => 0,
            Err(why) => {
                eprintln!("koma room attach: {why}");
                1
            }
        },
        RoomCli::Usage(why) => {
            eprintln!("koma room: {why}\n{USAGE}");
            2
        }
    }
}

mod attach {
    use std::io::{BufRead, Write};

    use tokio::sync::mpsc::unbounded_channel;

    use crate::ipc::client;
    use crate::ipc::frame::{read_frame_from, write_frame_to, FrameReader};
    use crate::ipc::split_stream;
    use crate::room::wire::{RoomClient, RoomServer};

    pub fn run(session: &str) -> Result<(), String> {
        let path = crate::model::store::daemon_sock_path(session).map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(drive(&path))
    }

    async fn drive(path: &std::path::Path) -> Result<(), String> {
        let stream = client::connect(path).await.map_err(|_| {
            format!("no secure daemon is listening on {} (start one with koma --daemon --kind secure)", path.display())
        })?;
        let (mut read, mut write) = split_stream(stream);
        let hello = serde_json::to_vec(&RoomClient::Hello).map_err(|e| e.to_string())?;
        write_frame_to(&mut write, &hello).await.map_err(|e| e.to_string())?;
        let mut reader = FrameReader::new();
        let first = read_frame_from(&mut read, &mut reader).await.map_err(|e| e.to_string())?;
        let welcome: RoomServer = serde_json::from_slice(&first).map_err(|e| e.to_string())?;
        match welcome {
            RoomServer::Hello { kind } if kind == "secure" => println!("room: secure"),
            RoomServer::Hello { kind } => return Err(format!("daemon kind is '{kind}', not secure")),
            _ => return Err("the daemon did not say hello".to_string()),
        }

        let (line_tx, mut line_rx) = unbounded_channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(text) => {
                        if line_tx.send(text).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut last_digest = String::new();
        loop {
            tokio::select! {
                line = line_rx.recv() => {
                    let Some(text) = line else {
                        let quit = serde_json::to_vec(&RoomClient::Quit).map_err(|e| e.to_string())?;
                        let _ = write_frame_to(&mut write, &quit).await;
                        break;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    let frame = serde_json::to_vec(&RoomClient::User { text }).map_err(|e| e.to_string())?;
                    write_frame_to(&mut write, &frame).await.map_err(|e| e.to_string())?;
                }
                incoming = read_frame_from(&mut read, &mut reader) => {
                    let bytes = incoming.map_err(|e| e.to_string())?;
                    let frame: RoomServer = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    if show(&frame, &mut last_digest) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// `true` when the daemon closed.
    fn show(frame: &RoomServer, last_digest: &mut String) -> bool {
        let mut out = std::io::stdout().lock();
        match frame {
            RoomServer::Hello { .. } => {}
            RoomServer::Chief { text } => { let _ = writeln!(out, "chief: {text}"); }
            RoomServer::LaneChat { avatar, text } => { let _ = writeln!(out, "{avatar}: {text}"); }
            RoomServer::Visible { author, text } => { let _ = writeln!(out, "{author}: {text}"); }
            RoomServer::Status { digest, .. } => {
                if digest != last_digest && !digest.is_empty() {
                    let _ = writeln!(out, "status: {digest}");
                    *last_digest = digest.clone();
                }
            }
            RoomServer::Closed => {
                let _ = writeln!(out, "room: closed");
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use serde_json::json;

    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("koma-room-cli-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("mkdir");
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        /// A session directory with a room database holding one claim.
        fn session(&self, bucket: &str, id: &str, claim: &str) -> PathBuf {
            let dir = self.0.join(bucket).join(id);
            std::fs::create_dir_all(&dir).expect("mkdir");
            let shelf = Shelf::open(&super::super::room_db_path(&dir)).expect("shelf");
            shelf
                .call(
                    "put_claim",
                    &json!({ "key": format!("claims/{claim}"), "vulnclass": "cors", "name": claim, "severity": "high",
                        "summary": "s", "host": "app.example", "poc": "curl", "expected_revision": 0 }),
                )
                .expect("claim");
            dir
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn defaults() -> TrajectoryArgs {
        TrajectoryArgs { goal: None, format: Format::Ascii, layered: false, width: None, session: None }
    }

    #[test]
    fn parses_trajectory_options() {
        assert_eq!(RoomCli::parse(&args(&["trajectory"])), RoomCli::Trajectory(defaults()));
        let parsed = RoomCli::parse(&args(&[
            "trajectory", "--goal", "web", "--format", "json", "--view", "layered", "--width", "140", "--session", "abc",
        ]));
        assert_eq!(
            parsed,
            RoomCli::Trajectory(TrajectoryArgs {
                goal: Some("web".into()), format: Format::Json, layered: true, width: Some(140), session: Some("abc".into()),
            })
        );
    }

    #[test]
    fn rejects_what_it_does_not_understand_with_a_reason() {
        let why = |list: &[&str]| match RoomCli::parse(&args(list)) {
            RoomCli::Usage(why) => why,
            other => panic!("expected usage, got {other:?}"),
        };
        assert!(why(&[]).contains("missing room command"));
        assert!(why(&["frobnicate"]).contains("unknown room command 'frobnicate'"));
        assert_eq!(RoomCli::parse(&args(&["attach", "--session", "abc"])), RoomCli::Attach { session: "abc".into() });
        assert!(why(&["attach"]).contains("--session"));
        assert!(why(&["attach", "--session", "../x"]).contains("not a session id"));
        assert!(why(&["trajectory", "--format", "svg"]).contains("unknown format 'svg'"));
        assert!(why(&["trajectory", "--view", "cube"]).contains("unknown view 'cube'"));
        assert!(why(&["trajectory", "--goal"]).contains("--goal needs a value"));
        assert!(why(&["trajectory", "--width", "5"]).contains("at least 20"));
        assert!(why(&["trajectory", "--width", "wide"]).contains("at least 20"));
        assert!(why(&["trajectory", "--bogus"]).contains("unknown option '--bogus'"));
    }

    #[test]
    fn picks_the_newest_room_session_for_the_directory() {
        let tmp = Tmp::new("newest");
        let older = tmp.session("bucket", "older", "a");
        std::thread::sleep(Duration::from_millis(20));
        let newer = tmp.session("bucket", "newer", "b");
        std::fs::create_dir_all(tmp.path().join("bucket").join("coding-only")).expect("mkdir");
        let bucket = tmp.path().join("bucket");
        assert_eq!(resolve_session(tmp.path(), &bucket, None).expect("session"), newer);
        assert_ne!(older, newer);
    }

    #[test]
    fn finds_a_named_session_here_or_in_another_directorys_bucket() {
        let tmp = Tmp::new("named");
        let here = tmp.session("bucket", "here", "a");
        let elsewhere = tmp.session("other-bucket", "there", "b");
        let bucket = tmp.path().join("bucket");
        assert_eq!(resolve_session(tmp.path(), &bucket, Some("here")).expect("here"), here);
        assert_eq!(resolve_session(tmp.path(), &bucket, Some("there")).expect("there"), elsewhere);
        assert!(resolve_session(tmp.path(), &bucket, Some("missing")).expect_err("missing").contains("no room session 'missing'"));
    }

    #[test]
    fn a_session_id_cannot_climb_out_of_the_sessions_directory() {
        let tmp = Tmp::new("escape");
        tmp.session("bucket", "real", "a");
        let bucket = tmp.path().join("bucket");
        for id in ["../bucket/real", "..", ".", "a/b", "a\\b"] {
            assert!(resolve_session(tmp.path(), &bucket, Some(id)).expect_err(id).contains("is not a session id"), "{id}");
        }
    }

    #[test]
    fn says_what_is_missing_when_there_is_no_room_session() {
        let tmp = Tmp::new("none");
        let bucket = tmp.path().join("bucket");
        assert!(resolve_session(tmp.path(), &bucket, None).expect_err("no bucket").contains("no sessions for this directory"));
        std::fs::create_dir_all(bucket.join("coding-only")).expect("mkdir");
        assert!(resolve_session(tmp.path(), &bucket, None).expect_err("no room").contains("no secure-room session"));
    }

    #[test]
    fn draws_the_trajectory_of_a_session() {
        let tmp = Tmp::new("draw");
        let db = super::super::room_db_path(&tmp.session("bucket", "s", "cors"));
        let text = render_trajectory(&db, &defaults(), 100).expect("render");
        assert!(text.starts_with("goal: "), "{text}");
        assert!(text.contains("claim: cors") && text.ends_with('\n'), "{text}");
        let json = render_trajectory(&db, &TrajectoryArgs { format: Format::Json, ..defaults() }, 100).expect("json");
        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok(), "{json}");
        let dot = render_trajectory(&db, &TrajectoryArgs { format: Format::Dot, ..defaults() }, 100).expect("dot");
        assert!(dot.starts_with("digraph"), "{dot}");
    }

    #[test]
    fn the_layered_view_gives_way_to_the_outline_when_the_width_is_small() {
        let tmp = Tmp::new("width");
        // a long label makes the graph wider than the smallest allowed width
        let db = super::super::room_db_path(&tmp.session("bucket", "s", "a-rather-long-claim-name-to-force-the-width"));
        let wide = render_trajectory(&db, &TrajectoryArgs { layered: true, ..defaults() }, 200).expect("render");
        assert!(wide.contains("─▶"), "{wide}");
        let narrow = render_trajectory(&db, &TrajectoryArgs { layered: true, width: Some(20), ..defaults() }, 200).expect("render");
        assert!(narrow.contains("└─ ") || narrow.contains("├─ "), "{narrow}");
    }

    #[test]
    fn reports_an_unknown_goal_and_an_empty_shelf() {
        let tmp = Tmp::new("errors");
        let db = super::super::room_db_path(&tmp.session("bucket", "s", "cors"));
        let missing = render_trajectory(&db, &TrajectoryArgs { goal: Some("nope".into()), ..defaults() }, 100);
        assert!(missing.expect_err("goal").contains("no goal 'nope'"));
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).expect("mkdir");
        Shelf::open(&super::super::room_db_path(&empty)).expect("empty shelf");
        let nothing = render_trajectory(&super::super::room_db_path(&empty), &defaults(), 100);
        assert!(nothing.expect_err("empty").contains("nothing on the shelf"));
        assert!(render_trajectory(&tmp.path().join("absent.sqlite"), &defaults(), 100).expect_err("absent").contains("no room database"));
    }

    #[test]
    fn viewing_a_session_never_changes_its_database() {
        let tmp = Tmp::new("readonly");
        let dir = tmp.session("bucket", "s", "cors");
        let db = super::super::room_db_path(&dir);
        // quiesce the writer so the file is stable, then compare it byte for byte
        drop(Shelf::open(&db).expect("reopen"));
        let before = std::fs::read(&db).expect("read");
        let files_before: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().map(|e| e.file_name()).collect();

        render_trajectory(&db, &defaults(), 100).expect("render");
        render_trajectory(&db, &TrajectoryArgs { format: Format::Json, ..defaults() }, 100).expect("render");

        assert_eq!(std::fs::read(&db).expect("read"), before, "the database file is untouched");
        let shelf = Shelf::open_read_only(&db).expect("open");
        assert_eq!(shelf.list_items(&[], false, 10).expect("list").len(), 1, "the data is still all there");
        // SQLite may add its WAL index files to read a WAL database; nothing else may appear.
        let files_after: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().map(|e| e.file_name()).collect();
        for file in files_after.iter().filter(|f| !files_before.contains(f)) {
            let name = file.to_string_lossy();
            assert!(name == "room.sqlite-wal" || name == "room.sqlite-shm", "unexpected new file {name}");
        }
    }

    #[test]
    fn the_read_only_shelf_refuses_writes_and_foreign_databases() {
        let tmp = Tmp::new("refuse");
        let db = super::super::room_db_path(&tmp.session("bucket", "s", "cors"));
        let shelf = Shelf::open_read_only(&db).expect("open read-only");
        assert!(shelf.list_items(&[], false, 10).expect("reads work").len() == 1);
        let write = shelf.call("put_scope", &json!({ "key": "scope/x", "host": "h", "expected_revision": 0 }));
        assert!(write.expect_err("write refused").0.to_lowercase().contains("readonly"));

        let foreign = tmp.path().join("foreign.sqlite");
        let conn = rusqlite::Connection::open(&foreign).expect("create");
        conn.execute_batch("CREATE TABLE other (x INTEGER);").expect("table");
        drop(conn);
        let refused = Shelf::open_read_only(&foreign).err().expect("a foreign database is refused");
        assert!(refused.0.contains("not a room database"), "{}", refused.0);
    }
}
