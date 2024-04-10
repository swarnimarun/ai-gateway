use std::path::PathBuf;

use async_trait::async_trait;
// use prometheus::register_int_counter;
use structopt::StructOpt;
use tracing::info;

use pingora_core::server::configuration::Opt;
use pingora_core::server::Server;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_http::ResponseHeader;
use pingora_proxy::{ProxyHttp, Session};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Model {
    ipaddr: String,
    port: u16,
    count_tokens: bool,
    inheader: String,
    outheader: String,
    search: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GatewaySpecification {
    enabled: Vec<String>,
    models: std::collections::HashMap<String, Model>,
}

fn files_at(path: PathBuf, files: &[&str]) -> Vec<PathBuf> {
    files.iter().map(|f| path.join(f)).collect()
}

impl GatewaySpecification {
    pub fn load_from_cwd() -> anyhow::Result<Self> {
        use anyhow::Context;
        let paths = files_at(std::env::current_dir()?, &[".aispec.toml", "aispec.toml"]);
        for path in paths {
            if let Ok(file) = std::fs::read_to_string(&path) {
                return toml::from_str(&file)
                    .with_context(|| format!("Failed to parse {}", path.display()));
            }
        }
        anyhow::bail!("no aispec.toml file found");
    }
}

pub struct MyGateway {
    specification: GatewaySpecification,
}

#[async_trait]
impl ProxyHttp for MyGateway {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut pingora_http::RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        // _session.req_header().headers.get("Host")
        upstream_request
            .insert_header("Host", "one.one.one.one")
            .unwrap();
        Ok(())
    }

    async fn request_filter(&self, session: &mut Session, _ctx: &mut Self::CTX) -> Result<bool> {
        if self.specification.enabled.len() == 0 {
            session.respond_error(501).await;
            return Ok(true);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(model) = session.req_header().headers.get("Model") {
            if let Ok(m) = model.to_str() {
                if let Some(m) = self.specification.models.get(m) {
                    let addr = (m.ipaddr.as_str(), m.port);
                    info!("connecting to {addr:?}");

                    let peer = Box::new(HttpPeer::new(addr, false, "one.one.one.one".to_string()));
                    return Ok(peer);
                }
            }
        }
        return Err(pingora_core::Error::new(pingora_core::ErrorType::Custom(
            "Failed to get address for upstream",
        )));
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()>
    where
        Self::CTX: Send + Sync,
    {
        // replace existing header if any
        // upstream_response
        //     .insert_header("Server", "MyGateway")
        //     .unwrap();
        // because we don't support h3
        upstream_response.remove_header("alt-svc");

        Ok(())
    }

    async fn logging(
        &self,
        session: &mut Session,
        _e: Option<&pingora_core::Error>,
        ctx: &mut Self::CTX,
    ) {
        let response_code = session
            .response_written()
            .map_or(0, |resp| resp.status.as_u16());
        info!(
            "{} response code: {response_code}",
            self.request_summary(session, ctx)
        );

        // self.req_metric.inc();
    }
}

// RUST_LOG=INFO cargo run --example load_balancer
// curl 127.0.0.1:6191 -H "Host: one.one.one.one"
// curl 127.0.0.1:6190/family/ -H "Host: one.one.one.one"
// curl 127.0.0.1:6191/login/ -H "Host: one.one.one.one" -I -H "Authorization: password"
// curl 127.0.0.1:6191/login/ -H "Host: one.one.one.one" -I -H "Authorization: bad"
// For metrics
// curl 127.0.0.1:6192/
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // read command line arguments
    let opt = Opt::from_args();
    let mut my_server = Server::new(Some(opt)).unwrap();
    my_server.bootstrap();

    let mut my_proxy = pingora_proxy::http_proxy_service(
        &my_server.configuration,
        MyGateway {
            specification: GatewaySpecification::load_from_cwd()
                .expect("failed to get AI Gateway Specification at cwd('.aispec.toml')"),
            // req_metric: register_int_counter!("reg_counter", "Number of requests").unwrap(),
        },
    );
    my_proxy.add_tcp("0.0.0.0:6191");
    my_server.add_service(my_proxy);

    // let mut prometheus_service_http =
    //     pingora_core::services::listening::Service::prometheus_http_service();
    // prometheus_service_http.add_tcp("127.0.0.1:6192");
    // my_server.add_service(prometheus_service_http);

    my_server.run_forever();
}
