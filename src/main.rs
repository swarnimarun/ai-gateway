use std::{collections::HashMap, path::PathBuf};

use async_trait::async_trait;
use structopt::StructOpt;

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
    models: HashMap<String, Model>,
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

pub struct AiGateway {
    specification: GatewaySpecification,
}

#[async_trait]
impl ProxyHttp for AiGateway {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

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
        let mut request: Vec<u8> = vec![];
        request.reserve(10240);
        while let Ok(Some(bytes)) = session.read_request_body().await {
            request.extend(&bytes[..]);
        }
        let json_body: serde_json::Value = serde_json::de::from_slice(&request).unwrap();
        if let Some(model_name) = json_body.get("model_name") {
            if let Some(mn) = model_name.as_str() {
                if let Some(model) = self.specification.models.get(mn) {
                    let addr = (model.ipaddr.as_str(), model.port);
                    // empty sni is fine as tls is set to "false"!
                    let sni = String::new();
                    let peer = Box::new(HttpPeer::new(addr, false, sni));
                    return Ok(peer);
                }
            }
        }
        panic!("unimplemented!");
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
        upstream_response
            .insert_header("Server", "MyGateway")
            .unwrap();

        // we don't support http3 yet, disable manually!
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
        tracing::info!(
            "{} response code: {response_code}",
            self.request_summary(session, ctx)
        );
    }
}

// RUST_LOG=INFO cargo run
// curl 127.0.0.1:6191 -H "Content-Type: application/json" --data '{"messages": [{"role": "user", "content": "Who is Newton?"}]}'
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

    let mut gateway = pingora_proxy::http_proxy_service(
        &my_server.configuration,
        AiGateway {
            specification: GatewaySpecification::load_from_cwd()
                .expect("failed to get AI Gateway Specification at cwd('.aispec.toml')"),
        },
    );
    gateway.add_tcp("0.0.0.0:6191");
    my_server.add_service(gateway);

    // let mut prometheus_service_http =
    //     pingora_core::services::listening::Service::prometheus_http_service();
    // prometheus_service_http.add_tcp("127.0.0.1:6192");
    // my_server.add_service(prometheus_service_http);

    my_server.run_forever();
}
