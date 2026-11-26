use anyhow::{Context, Result};
use bedrock_foundry::{backend::Backend, marketplace::CatalogQuery, server};
use clap::{Parser, Subcommand};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::atomic::AtomicBool,
};
use uuid::Uuid;

#[cfg(feature = "desktop")]
mod desktop;

#[derive(Parser)]
#[command(
    version,
    about = "Set up Bedrock servers with your Marketplace add-ons."
)]
struct Cli {
    /// Use a separate application data directory.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Connect or disconnect a Microsoft account.
    Account {
        #[command(subcommand)]
        action: AccountCommand,
    },
    /// List, claim, and download Marketplace add-ons.
    Addon {
        #[command(subcommand)]
        action: AddonCommand,
    },
    /// Browse and search Marketplace listings or read your Minecoin balance.
    Marketplace {
        #[command(subcommand)]
        action: MarketplaceCommand,
    },
    /// Create and manage Bedrock server instances.
    Server {
        #[command(subcommand)]
        action: ServerCommand,
    },
}
#[derive(Subcommand)]
enum AccountCommand {
    Login,
    Logout,
    Status,
    /// Import a local `MinecraftAuth` account without printing credentials.
    Import {
        path: PathBuf,
    },
}
#[derive(Subcommand)]
enum AddonCommand {
    List,
    Sample,
    /// Claim a zero-Minecoin add-on. Paid purchases are refused.
    Claim {
        id: Uuid,
    },
    Download {
        id: Uuid,
    },
    /// Review an add-on's price. Spending requires --price and --confirm.
    Buy {
        id: Uuid,
        #[arg(long)]
        price: Option<u64>,
        #[arg(long, requires = "price")]
        confirm: bool,
    },
}
#[derive(Subcommand)]
enum MarketplaceCommand {
    Browse {
        #[arg(long, default_value = "")]
        search: String,
        #[arg(long)]
        addons: bool,
        #[arg(long, default_value_t = 0)]
        skip: u64,
        #[arg(long)]
        json: bool,
    },
    Balance,
}
#[derive(Subcommand)]
enum ServerCommand {
    List {
        /// Print the servers as a JSON array.
        #[arg(long)]
        json: bool,
    },
    Create {
        name: String,
        #[arg(long, default_value_t = 19132)]
        port: u16,
    },
    Install {
        id: Uuid,
    },
    /// Import an official BDS ZIP when automatic download is unavailable.
    Import {
        id: Uuid,
        archive: PathBuf,
        #[arg(long)]
        version: String,
    },
    Addon {
        id: Uuid,
        addon: Uuid,
    },
    /// Run BDS in the foreground. Enter console commands or press Ctrl+C to stop.
    Run {
        id: Uuid,
        /// For programs: print JSON Lines events, and stop the server when input closes.
        #[arg(long)]
        json: bool,
    },
    Backup {
        id: Uuid,
    },
    /// Accept network connections. Firewall and router rules remain user-controlled.
    Expose {
        id: Uuid,
        #[arg(long)]
        local: bool,
        #[arg(long, conflicts_with = "local")]
        address: Option<std::net::IpAddr>,
    },
    Folder {
        id: Uuid,
    },
}
/// Runs a server until it exits. In JSON mode, closing standard input stops the server, so a
/// controlling program that exits or crashes cannot leave BDS running without an owner.
async fn run_server(backend: &Backend, id: Uuid, json: bool) -> Result<()> {
    let config = backend.server(id)?;
    let port = config.port;
    let processes = backend.processes.clone();
    let root = backend.store.root.clone();
    tokio::task::spawn_blocking(move || processes.start(&root, &config)).await??;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin()
            .lock()
            .lines()
            .map_while(std::result::Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let emit = |event: serde_json::Value| -> Result<()> {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{event}")?;
        out.flush()?;
        Ok(())
    };
    let mut last = Vec::new();
    let mut ready = false;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut input_open = true;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {stop(backend, id).await?; break;},
            command = rx.recv(), if input_open => match command {
                Some(text) => backend.processes.command(id, &text)?,
                None if json => {stop(backend, id).await?; input_open = false;},
                None => input_open = false,
            },
            _ = tick.tick() => {
                let logs = backend.processes.logs(id);
                let overlap = last.len().min(logs.len());
                let start = (0..=overlap).rev().find(|n| last[last.len() - n..] == logs[..*n]).unwrap_or(0);
                for line in &logs[start..] {
                    if json {
                        emit(serde_json::json!({"event": "log", "line": line}))?;
                        if !ready && server::is_ready_line(line) {
                            ready = true;
                            emit(serde_json::json!({"event": "ready", "host": "127.0.0.1", "port": port}))?;
                        }
                    } else {
                        println!("{line}");
                    }
                }
                std::io::stdout().flush()?;
                last = logs;
                if !backend.processes.running(id)? {break;}
            }
        }
    }
    if json {
        emit(serde_json::json!({"event": "stopped"}))?;
    }
    Ok(())
}
/// Stopping waits up to ten seconds for BDS to save and exit, so it runs off the async runtime.
async fn stop(backend: &Backend, id: Uuid) -> Result<()> {
    let processes = backend.processes.clone();
    tokio::task::spawn_blocking(move || processes.stop(id)).await?
}
fn main() -> std::process::ExitCode {
    // An existing global subscriber can continue to handle logging.
    drop(
        tracing_subscriber::fmt()
            .with_env_filter("warn,gpui=info")
            .with_ansi(false)
            .try_init(),
    );
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.command.is_none() {
        #[cfg(feature = "desktop")]
        {
            desktop::launch(cli.data_dir);
            return Ok(());
        }
        #[cfg(not(feature = "desktop"))]
        {
            anyhow::bail!(
                "This build has no desktop interface. Run bedrock-foundry --help for CLI commands."
            );
        }
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let mut backend = Backend::open(cli.data_dir)?;
    runtime.block_on(async {
        match cli.command.context("Missing command")? {
            Command::Account{action}=>match action {
                AccountCommand::Login=>{let cancel=AtomicBool::new(false);let account=backend.accounts.sign_in(|url,code|println!("Open {url} and enter {code}"),&cancel).await?;println!("Connected as {}",account.gamertag);},
                AccountCommand::Logout=>{backend.accounts.sign_out()?;println!("Account disconnected");},
                AccountCommand::Status=>{match backend.restore().await? {Some(a)=>println!("Connected as {}",a.gamertag),None=>println!("No account connected")}},
                AccountCommand::Import{path}=>{backend.accounts.import_legacy(&path)?;println!("Connected as {}",backend.accounts.info().await?.gamertag);},
            },
            Command::Addon{action}=>{
                backend.restore().await?;
                match action {
                    AddonCommand::List=>for addon in backend.refresh_library().await? {println!("{}  {}",addon.id,addon.name);},
                    AddonCommand::Sample=>{let addon=backend.sample().await?;println!("{}  {}  {}",addon.id,addon.name,if addon.owned{"Owned"}else{"Free"});},
                    AddonCommand::Claim{id}=>{backend.claim(id).await?;println!("Claim accepted. Refresh your library to see the entitlement.");},
                    AddonCommand::Download{id}=>{backend.refresh_library().await?;backend.download(id).await?;println!("Add-on downloaded");},
                    AddonCommand::Buy{id,price,confirm}=>{
                        let quote=backend.quote_purchase(id).await?;
                        println!("{}: {} Minecoins. Balance: {} Minecoins.",quote.item.title,quote.amount,quote.balance);
                        if confirm {
                            anyhow::ensure!(price==Some(quote.amount),"The quoted price differs from --price. Review the purchase again.");
                            let ready=backend.buy(quote.id).await?;
                            println!("{}",if ready{"Purchase accepted. The add-on is ready to download."}else{"Purchase accepted. Refresh your library to check entitlement delivery. Do not repeat the purchase."});
                        } else {
                            println!("No purchase made. To confirm: addon buy {id} --price {} --confirm",quote.amount);
                        }
                    },
                }
            },
            Command::Marketplace{action}=>{
                backend.restore().await?;
                match action {
                    MarketplaceCommand::Balance=>println!("{} Minecoins",backend.refresh_balance().await?),
                    MarketplaceCommand::Browse{search,addons,skip,json}=>{
                        backend.search_catalog(CatalogQuery{search,addons_only:addons,skip}).await?;
                        let page=backend.catalog.as_ref().context("Catalog did not load")?;
                        if json {println!("{}",serde_json::to_string_pretty(page)?);} else {
                            for item in &page.items {println!("{}  {}  [{}]  {}",item.id,item.title,item.kind,item.price_label());}
                            println!("{} listings. Total: {}.",page.items.len(),page.total);
                            if let Some(next)=page.next_skip {println!("Next page: repeat this query with --skip {next}");}
                        }
                    }
                }
            },
            Command::Server{action}=>match action {
                ServerCommand::List{json}=>{
                    let servers=backend.servers()?;
                    if json {println!("{}",serde_json::to_string_pretty(&servers.iter().map(server::Summary::from).collect::<Vec<_>>())?);}
                    else {for config in servers {println!("{}  {}  port {}  {}",config.id,config.name,config.port,config.version.as_deref().unwrap_or("BDS not installed"));}}
                },
                ServerCommand::Create{name,port}=>{let server=backend.create(name,port)?;println!("{}",server.id);},
                ServerCommand::Install{id}=>{backend.provision(id).await?;println!("BDS installed");},
                ServerCommand::Import{id,archive,version}=>{let mut config=backend.server(id)?;let metadata=tokio::fs::metadata(&archive).await?;anyhow::ensure!(metadata.len()<=bedrock_foundry::archive::MAX_ARCHIVE as u64,"Server archive exceeds the size limit");let bytes=tokio::fs::read(archive).await?;let root=backend.store.root.clone();tokio::task::spawn_blocking(move || server::import_distribution(&root,&mut config,&bytes,version)).await??;println!("BDS imported");},
                ServerCommand::Addon{id,addon}=>{backend.install(id,addon)?;println!("Add-on installed");},
                ServerCommand::Backup{id}=>println!("{}",backend.backup(id)?.display()),
                ServerCommand::Expose{id,local,address}=>{let mut config=backend.server(id)?;config.public = !local;config.public_address=address;backend.update(&config)?;println!("Network binding: {}:{}",if local{"127.0.0.1"}else{"0.0.0.0"},config.port);},
                ServerCommand::Folder{id}=>println!("{}",backend.folder(id).display()),
                ServerCommand::Run{id,json}=>run_server(&backend,id,json).await?,
            },
        }
        Ok(())
    })
}
