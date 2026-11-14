# Use the command line

Run `bedrock-foundry` without a command to open the desktop. Use `--help` after a
command to see its arguments. Examples below use the executable name. During
development, replace it with `cargo run --locked --`.

## Connect an account

```sh
bedrock-foundry account login
bedrock-foundry account status
bedrock-foundry account logout
```

Sign-in prints a Microsoft verification page and a short code. Credentials stay
in the OS credential store. `account import PATH` accepts an existing local
MinecraftAuth Bedrock JSON account for migration. The import does not print its
tokens or copy that file into the repository.

## Download add-ons

```sh
bedrock-foundry addon list
bedrock-foundry addon sample
bedrock-foundry addon claim ADDON_ID
bedrock-foundry addon download ADDON_ID
```

`sample` shows the free Poisonous Potato Add-On. `claim` refuses offers whose
current catalog price is not zero. Foundry does not make paid purchases. Buy paid
add-ons through Minecraft before refreshing the owned library.

## Create and run a server

```sh
bedrock-foundry server create "Friends" --port 19132
bedrock-foundry server list
bedrock-foundry server install SERVER_ID
bedrock-foundry server addon SERVER_ID ADDON_ID
bedrock-foundry server run SERVER_ID
```

`create` prints the server ID. `run` keeps the server in the foreground and
forwards each line of terminal input to its console. Enter `stop` or press Ctrl+C
to request shutdown. If BDS does not exit within ten seconds, Foundry terminates
its owned process. Closing the desktop also requests shutdown of its servers.

When an automatic download is unavailable, import an official ZIP:

```sh
bedrock-foundry server import SERVER_ID /path/to/bedrock-server-VERSION.zip --version VERSION
```

Import refuses an instance that already contains BDS. It preserves the managed
configuration and does not replace an existing world or account allowlist.
The desktop import expects the original `bedrock-server-VERSION.zip` filename.

## Back up and configure access

```sh
bedrock-foundry server backup SERVER_ID
bedrock-foundry server folder SERVER_ID
bedrock-foundry server expose SERVER_ID
bedrock-foundry server expose SERVER_ID --address PUBLIC_IP
bedrock-foundry server expose SERVER_ID --local
```

`backup` prints the backup ZIP path. `folder` prints the instance directory.
Exposure changes take effect on the next start. Read the [hosting guide](hosting.md)
before forwarding ports. Foundry does not change router or firewall rules.

## Keep separate installations

Use `--data-dir PATH` or `FOUNDRY_HOME` to select a separate application directory.
The account's keychain entry is scoped to that directory. Foundry resolves the
path before selecting the entry, so relative paths and aliases share one owner.

The desktop and CLI cannot use the same data directory at the same time. Close
the desktop before running CLI commands against its instances. Different data
directories can run at the same time, but their server ports must remain distinct.
