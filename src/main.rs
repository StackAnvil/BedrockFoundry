use anyhow::{Context, Result};
use bedrock_foundry::{backend::Backend, server};
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
    /// Import a local MinecraftAuth account without printing credentials.
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
}
#[derive(Subcommand)]
enum ServerCommand {
    List,
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
fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,gpui=info")
        .with_ansi(false)
        .try_init();
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.command.is_none() {
        #[cfg(feature = "desktop")]
        {
            return desktop::launch(cli.data_dir);
        }
        #[cfg(not(feature = "desktop"))]
        {
            anyhow::bail!(
                "This build has no desktop interface. Run bedrock-foundry --help for CLI commands."
            );
        }
    }
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let mut backend=Backend::open(cli.data_dir)?;
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
                }
            },
            Command::Server{action}=>match action {
                ServerCommand::List=>for config in backend.servers()? {println!("{}  {}  port {}  {}",config.id,config.name,config.port,config.version.as_deref().unwrap_or("BDS not installed"));},
                ServerCommand::Create{name,port}=>{let server=backend.create(name,port)?;println!("{}",server.id);},
                ServerCommand::Install{id}=>{backend.provision(id).await?;println!("BDS installed");},
                ServerCommand::Import{id,archive,version}=>{let mut config=backend.server(id)?;let metadata=std::fs::metadata(&archive)?;anyhow::ensure!(metadata.len()<=bedrock_foundry::archive::MAX_ARCHIVE as u64,"Server archive exceeds the size limit");server::import_distribution(&backend.store.root,&mut config,&std::fs::read(archive)?,version)?;println!("BDS imported");},
                ServerCommand::Addon{id,addon}=>{backend.install(id,addon)?;println!("Add-on installed");},
                ServerCommand::Backup{id}=>println!("{}",backend.backup(id)?.display()),
                ServerCommand::Expose{id,local,address}=>{let mut config=backend.server(id)?;config.public = !local;config.public_address=address;backend.update(&config)?;println!("Network binding: {}:{}",if local{"127.0.0.1"}else{"0.0.0.0"},config.port);},
                ServerCommand::Folder{id}=>println!("{}",backend.folder(id).display()),
                ServerCommand::Run{id}=>{
                    backend.start(id)?;
                    let (tx,mut rx)=tokio::sync::mpsc::unbounded_channel();
                    std::thread::spawn(move || {for line in std::io::stdin().lock().lines().map_while(std::result::Result::ok) {if tx.send(line).is_err(){break;}}});
                    let mut last=Vec::new();let mut tick=tokio::time::interval(std::time::Duration::from_millis(100));let mut input_open=true;
                    loop {
                        tokio::select! {
                            _=tokio::signal::ctrl_c()=>{backend.processes.stop(id)?;break;},
                            command=rx.recv(),if input_open=>{match command {Some(text)=>backend.processes.command(id,&text)?,None=>input_open=false}},
                            _=tick.tick()=>{
                                let logs=backend.processes.logs(id);
                                let overlap=last.len().min(logs.len());let start=(0..=overlap).rev().find(|n|last[last.len()-n..]==logs[..*n]).unwrap_or(0);
                                for line in &logs[start..]{println!("{line}");}
                                std::io::stdout().flush()?;last=logs;
                                if !backend.processes.running(id){break;}
                            }
                        }
                    }
                },
            },
        }
        Ok(())
    })
}
