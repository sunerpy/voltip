//! A TCP proxy between a node and the relay whose path breaks without a word, as it does when a
//! phone changes networks or a cross-border path drops (docs/pairing.md 「重连」): after
//! [`NetCut::cut`] the connections open at that moment carry nothing either way and are never
//! closed, so both ends see a socket that is up and silent; a connection opened afterwards (the
//! device's next one, over its new network) carries traffic as before.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

pub struct NetCut {
    addr: SocketAddr,
    epoch: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl NetCut {
    /// Listen on an ephemeral loopback port and forward every connection to `upstream`.
    pub async fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let epoch = Arc::new(AtomicU64::new(0));
        let shared = epoch.clone();
        let task = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                let born = shared.load(Ordering::SeqCst);
                let epoch = shared.clone();
                tokio::spawn(async move {
                    let Ok(up) = TcpStream::connect(upstream).await else { return };
                    let (down_read, down_write) = down.into_split();
                    let (up_read, up_write) = up.into_split();
                    tokio::spawn(pump(down_read, up_write, epoch.clone(), born));
                    tokio::spawn(pump(up_read, down_write, epoch, born));
                });
            }
        });
        Self { addr, epoch, task }
    }

    /// The relay URL a node uses to go through this proxy.
    pub fn url(&self) -> String {
        format!("ws://{}/ws", self.addr)
    }

    /// Break the path of every connection open now.
    pub fn cut(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for NetCut {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Copy `from` to `to` until the path is cut, then hold both halves and swallow everything: data,
/// and the close of the far end too (a relay that closes a silent connection is not heard either).
async fn pump(mut from: tokio::net::tcp::OwnedReadHalf, mut to: tokio::net::tcp::OwnedWriteHalf, epoch: Arc<AtomicU64>, born: u64) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let read = from.read(&mut buf).await;
        if epoch.load(Ordering::SeqCst) != born {
            let _held = (from, to);
            std::future::pending::<()>().await;
            return;
        }
        let n = match read {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        if to.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}
