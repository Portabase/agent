use crate::services::config::DatabaseConfig;
use anyhow::Result;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub async fn build_client(cfg: &DatabaseConfig) -> Result<Client<Compat<TcpStream>>> {
    let mut config = Config::new();
    config.host(&cfg.host);
    config.port(cfg.port);
    config.authentication(AuthMethod::sql_server(&cfg.username, &cfg.password));
    config.trust_cert();

    let tcp = TcpStream::connect(config.get_addr()).await?;
    tcp.set_nodelay(true)?;
    let client = Client::connect(config, tcp.compat_write()).await?;
    Ok(client)
}
