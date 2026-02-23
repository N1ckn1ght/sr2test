/// Static application
/// 
/// TODO: add postgre db using sqlx

use std::{collections::HashSet, convert::Infallible, net::{IpAddr, SocketAddr}, sync::Arc, time::Duration};
use log::{Level, info, warn};
use tokio::{fs, net::TcpListener, sync::Mutex, time};
use hyper_util::rt::TokioIo;
use hyper::{Method, Request, Response, StatusCode, body::{Bytes, Incoming}, service::service_fn};
use hyper::server::conn::http1;
use http_body_util::Full;


const WINDOW: Duration = Duration::from_secs(15);


#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> std::io::Result<()> {
    simple_logger::init_with_level(Level::Info).unwrap();

    println!("Hello, world!");

    let active_ips = Arc::new(Mutex::new(HashSet::new()));
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    loop {
        let (stream, addr) = listener.accept().await?;
        let active_ips = Arc::clone(&active_ips);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let conn = http1::Builder::new().serve_connection(io, service_fn(move |req| {
                handler(req, addr, Arc::clone(&active_ips))
            }));
            info!("{}\tconnected", addr);

            let _ = conn.await;
        });
    }
}

async fn handler(req: Request<Incoming>, addr: SocketAddr, active_ips: Arc<Mutex<HashSet<IpAddr>>>) -> Result<Response<Full<Bytes>>, Infallible> {
    let ip = addr.ip();
    if active_ips.lock().await.contains(&ip) {
        info!("{}\t\tdebounced", &ip);
        return Ok(Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(Full::new(Bytes::from("429 too many requests")))
            .unwrap());
    }
    active_ips.lock().await.insert(addr.ip());
    let _bouncer = Bouncer::new(ip, Arc::clone(&active_ips));
    info!("{}\t\tsaved", addr.ip());

    match time::timeout(WINDOW, router(req)).await {
        Ok(response) => {
            info!("{}\tsucceded", addr);
            active_ips.lock().await.remove(&addr.ip());
            info!("{}\t\treleased", addr.ip());
            response
        },
        Err(_) => {
            warn!("{}\ttimed out", addr);
            active_ips.lock().await.remove(&addr.ip());
            info!("{}\t\treleased", addr.ip());
            Ok(Response::builder()
                .status(StatusCode::REQUEST_TIMEOUT)
                .body(Full::new(Bytes::from("408 timed out")))
                .unwrap())
        }
    }
}

async fn router(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/") => {
            let body = fs::read_to_string("src/website/index.html").await.unwrap_or_else(|_| "placeholder".into());
            Ok(Response::new(Full::new(Bytes::from(body))))
        }
        (&Method::GET, "/sleep") => {
            time::sleep(Duration::from_secs(10)).await;
            let body = fs::read_to_string("src/website/index.html").await.unwrap_or_else(|_| "placeholder".into());
            Ok(Response::new(Full::new(Bytes::from(body))))
        }
        (&Method::GET, "/timeout") => {
            time::sleep(Duration::from_secs(20)).await;
            let body = fs::read_to_string("src/website/index.html").await.unwrap_or_else(|_| "placeholder".into());
            Ok(Response::new(Full::new(Bytes::from(body))))
        }
        _ => {
            let body = fs::read_to_string("src/website/404.html").await.unwrap_or_else(|_| "404 not found".into());
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from(body)))
                .unwrap())
        }
    }
}


// spawn task on drop to drop IP
struct Bouncer {
    ip: IpAddr,
    active_ips: Arc<Mutex<HashSet<IpAddr>>>,
}

impl Bouncer {
    fn new(ip: IpAddr, active_ips: Arc<Mutex<HashSet<IpAddr>>>) -> Self {
        Self { 
            ip, 
            active_ips 
        }
    }
}

impl Drop for Bouncer {
    fn drop(&mut self) {
        let active_ips = Arc::clone(&self.active_ips);
        let ip = self.ip;
        tokio::spawn(async move {
            let mut set = active_ips.lock().await;
            if set.remove(&ip) {
                info!("{}\t\treleased", ip)
            } else {
                info!("{}\t\treleased already", ip);
            }
        });
    }
}
