use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

pub const LINE_LIMIT: usize = 1024 * 1024;
const SOCKET_PATH_LIMIT: usize = 100;
const SOCKET_FILE_LENGTH: usize = 13;
const HELLO_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    Request {
        conn: u64,
        id: u64,
        tool: String,
        input: Value,
        tool_use_id: Option<String>,
    },
    Closed {
        conn: u64,
    },
}

type Writers = Arc<Mutex<HashMap<u64, OwnedWriteHalf>>>;

pub struct Channel {
    path: PathBuf,
    token: String,
    inbound: mpsc::UnboundedReceiver<Inbound>,
    writers: Writers,
    accepter: JoinHandle<()>,
}

pub async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<String>> {
    buffer.clear();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(None);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if buffer.len() + take > LINE_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        buffer.extend_from_slice(&available[..take]);
        reader.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(String::from_utf8_lossy(buffer).into_owned()));
        }
    }
}

async fn serve(
    stream: UnixStream,
    conn: u64,
    token: String,
    writers: Writers,
    inbound: mpsc::UnboundedSender<Inbound>,
) {
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut buffer = Vec::new();
    let hello = timeout(HELLO_WAIT, read_bounded_line(&mut reader, &mut buffer)).await;
    let accepted = matches!(&hello, Ok(Ok(Some(line)))
        if serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value.get("hello").and_then(Value::as_str).map(str::to_owned))
            .is_some_and(|sent| sent == token));
    if !accepted {
        return;
    }
    writers.lock().await.insert(conn, write);
    while let Ok(Some(line)) = read_bounded_line(&mut reader, &mut buffer).await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (Some(id), Some(tool), Some(input)) = (
            value.get("id").and_then(Value::as_u64),
            value.get("tool").and_then(Value::as_str),
            value.get("input").filter(|input| input.is_object()),
        ) else {
            continue;
        };
        let request = Inbound::Request {
            conn,
            id,
            tool: tool.to_owned(),
            input: input.clone(),
            tool_use_id: value
                .get("toolUseId")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        if inbound.send(request).is_err() {
            return;
        }
    }
    writers.lock().await.remove(&conn);
    let _ = inbound.send(Inbound::Closed { conn });
}

pub fn socket_dir(dir: &Path) -> PathBuf {
    if dir.as_os_str().len() + 1 + SOCKET_FILE_LENGTH <= SOCKET_PATH_LIMIT {
        return dir.to_path_buf();
    }
    let mut hasher = DefaultHasher::new();
    dir.hash(&mut hasher);
    std::env::temp_dir().join(format!("shikigami-ap-{:08x}", hasher.finish() as u32))
}

fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let metadata = std::fs::metadata(dir)?;
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the socket folder belongs to someone else",
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

impl Channel {
    pub fn bind(dir: &Path) -> io::Result<Self> {
        let dir = socket_dir(dir);
        let dir = dir.as_path();
        std::fs::create_dir_all(dir)?;
        if dir
            .parent()
            .is_some_and(|parent| parent == std::env::temp_dir())
        {
            private_dir(dir)?;
        }
        let name = uuid::Uuid::new_v4().simple().to_string();
        let path = dir.join(format!("ap-{}.s", &name[..8]));
        let listener = UnixListener::bind(&path)?;
        if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        let writers: Writers = Arc::default();
        let (sender, inbound) = mpsc::unbounded_channel();
        let accepter = tokio::spawn(accept_loop(
            listener,
            token.clone(),
            Arc::clone(&writers),
            sender,
        ));
        Ok(Self {
            path,
            token,
            inbound,
            writers,
            accepter,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub async fn next(&mut self) -> Option<Inbound> {
        self.inbound.recv().await
    }

    pub async fn reply(&self, conn: u64, id: u64, decision: Value) -> bool {
        let mut line = json!({"id": id});
        if let (Some(target), Some(fields)) = (line.as_object_mut(), decision.as_object()) {
            target.extend(fields.clone());
        }
        let mut text = line.to_string();
        text.push('\n');
        let mut writers = self.writers.lock().await;
        let Some(writer) = writers.get_mut(&conn) else {
            return false;
        };
        if writer.write_all(text.as_bytes()).await.is_err() {
            writers.remove(&conn);
            return false;
        }
        true
    }
}

async fn accept_loop(
    listener: UnixListener,
    token: String,
    writers: Writers,
    inbound: mpsc::UnboundedSender<Inbound>,
) {
    let mut connections = JoinSet::new();
    let mut next_conn = 0u64;
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        next_conn += 1;
        connections.spawn(serve(
            stream,
            next_conn,
            token.clone(),
            Arc::clone(&writers),
            inbound.clone(),
        ));
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.accepter.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}
