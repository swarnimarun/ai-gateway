use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
// use prometheus::register_int_counter;
use structopt::StructOpt;
use tokenizers::Tokenizer;
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
    host: String,
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
    tokenizer: tokenizers::Tokenizer,
    specification: GatewaySpecification,
}

pub struct Ctx {
    buffer: Vec<u8>,
    input_token_count: usize,
}

impl MyGateway {
    pub fn get_token_count(&self, s: &str) -> usize {
        let res = self.tokenizer.encode(s, false).unwrap();
        res.len()
    }
}

#[async_trait]
impl ProxyHttp for MyGateway {
    type CTX = Ctx;
    fn new_ctx(&self) -> Self::CTX {
        Ctx {
            buffer: vec![],
            input_token_count: 0,
        }
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut pingora_http::RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(model) = session.req_header().headers.get("model") {
            upstream_request
                .insert_header(
                    "Host",
                    self.specification.models[model.to_str().unwrap()]
                        .host
                        .as_str(),
                )
                .unwrap();
        }
        Ok(())
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        if self.specification.enabled.len() == 0 {
            session.respond_error(501).await;
            return Ok(true);
        }
        let mut req_body = vec![];
        while let Ok(Some(bytes)) = session.read_request_body().await {
            req_body.extend(bytes);
        }
        let req_body: serde_json::error::Result<serde_json::Value> =
            serde_json::de::from_slice(&req_body);
        if let Ok(val) = req_body {
            if let Some(v) = val.as_object() {
                let src = recursive_search_all(v, "content");
                ctx.input_token_count = self.get_token_count(&src);
                return Ok(false);
            }
        }
        session.respond_error(501).await;
        Ok(true)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(model) = session.req_header().headers.get("model") {
            if let Ok(m) = model.to_str() {
                if let Some(m) = self.specification.models.get(m) {
                    let addr = (m.ipaddr.as_str(), m.port);
                    info!("connecting to {addr:?}");

                    // TODO: support TLS
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
        // because we don't support h3
        upstream_response.remove_header("alt-svc");

        upstream_response.remove_header("Content-Length");
        upstream_response
            .insert_header("Transfer-Encoding", "Chunked")
            .unwrap();

        Ok(())
    }

    fn response_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<std::time::Duration>>
    where
        Self::CTX: Send + Sync,
    {
        // buffer the data
        if let Some(b) = body {
            ctx.buffer.extend(&b[..]);
            // drop the body
            b.clear();
        }
        if end_of_stream {
            info!("end of stream");
            // This is the last chunk, we can process the data now
            let mut json_body: serde_json::Value = serde_json::de::from_slice(&ctx.buffer).unwrap();
            if let Some(value) = session.req_header().headers.get("model") {
                if let Some(jb) = json_body.as_object_mut() {
                    let content = recursive_search_all(
                        jb,
                        &self.specification.models[value.to_str().unwrap()].outheader,
                    );
                    let tok_c = self.get_token_count(&content);
                    jb.insert("output_token_count".to_string(), tok_c.into());
                    jb.insert(
                        "input_token_count".to_string(),
                        ctx.input_token_count.into(),
                    );
                    *body = Some(Bytes::copy_from_slice(json_body.to_string().as_bytes()));
                }
            }
        }
        Ok(None)
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
    }
}

pub fn recursive_search_all(
    m: &serde_json::Map<String, serde_json::Value>,
    key: impl AsRef<str>,
) -> String {
    let mut ret = String::new();
    for (k, v) in m {
        if k == key.as_ref() {
            v.as_str().map(|s| ret.push_str(s));
        } else if v.is_object() {
            if let Some(o) = v.as_object() {
                let src = recursive_search_all(o, key.as_ref());
                ret.push_str(&src);
            }
        }
    }
    ret
}

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
            tokenizer: Tokenizer::from_pretrained("bert-base-cased", None)
                .expect("failed to get bert base cased tokenizer"),
            specification: GatewaySpecification::load_from_cwd()
                .expect("failed to get AI Gateway Specification at cwd('.aispec.toml')"),
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
