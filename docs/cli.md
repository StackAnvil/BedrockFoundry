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
current catalog price is not zero.

## Browse the Marketplace

```sh
bedrock-foundry marketplace browse
bedrock-foundry marketplace browse --search dragons
bedrock-foundry marketplace browse --addons --search furniture
bedrock-foundry marketplace browse --addons --skip 24 --json
bedrock-foundry marketplace balance
```

Browsing returns one page of public listings. Repeat the same query with the
printed next offset to see more. `--json` includes descriptions, prices, ownership,
and the next offset. Other product types are viewable but cannot be bought or
installed here.

Review an add-on without spending Minecoins:

```sh
bedrock-foundry addon buy ADDON_ID
```

To buy it, repeat the command with its exact reviewed price and `--confirm`:

```sh
bedrock-foundry addon buy ADDON_ID --price 490 --confirm
```

Replace `490` with the quoted price. The app rejects a changed offer, expired quote,
different account, or insufficient balance. It re-reads the offer and balance before
submitting. Minecoin purchases cannot be undone through Foundry.

After an unknown outcome, do not repeat the purchase. Refresh `addon list` and check
the purchase in Minecraft. Foundry retains a record for that account and offer and
blocks another attempt until ownership is confirmed. An accepted purchase remains
accepted if the later library refresh fails. Refresh the library to collect it.

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

## Control a server from another program

Programs such as the StackAnvil companion mod use two JSON forms:

```sh
bedrock-foundry server list --json
bedrock-foundry server run SERVER_ID --json
```

`server list --json` prints an array. Each server has `id`, `name`, `port`,
`version` (`null` before BDS is installed), `public`, and `addons` with each
add-on's `id` and `name`.

`server run --json` prints one JSON object per line:

- `{"event":"log","line":"..."}` for each BDS console line.
- `{"event":"ready","host":"127.0.0.1","port":19132}` once, when BDS reports
  `Server started.` Players can join that address.
- `{"event":"stopped"}` after BDS exits.

Each input line is a console command. When input closes, Foundry stops the
server. A controlling program that exits or crashes closes this input, so BDS
does not keep running without an owner. Errors before start go to standard error
with a nonzero exit code.

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
