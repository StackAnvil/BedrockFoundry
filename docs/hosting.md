# Connect to a Foundry server

Foundry configures BDS to use NetherNet. This is the supported transport in the
BDS builds tested by this project. Current hosting instructions must account for
both the signaling connection and the UDP gameplay transport.

## Connect from this computer

Start the server and wait for **Running**. Use the address `127.0.0.1` and the
port shown in Foundry. The default signaling port is 19132. Foundry reserves
19133 for UDP gameplay. Microsoft authentication stays enabled on the server.
BDS needs access to Minecraft services during startup.

Foundry's running indicator means that BDS reported successful startup. It does
not prove that a particular game client completed a join or received every pack.

## Accept connections from another computer

1. Stop the server.
2. Select **Allow network connections**.
3. Allow inbound TCP on the selected server port in the host firewall.
4. Allow inbound UDP on the next port.
5. Start the server and wait for **Running**.
6. Give the other player the host's LAN address and the selected server port.

For the default configuration, allow TCP 19132 and UDP 19133. Other instances
need distinct port pairs. The server binds to all IPv4 interfaces after you enable
network connections. **Use this computer only** restores the loopback binding.

## Forward a public address

Configure the router to forward the signaling TCP port and gameplay UDP port to
the computer that runs BDS. Keep the external and internal ports the same.
Publish the public IP in the UDP candidate mapping with the CLI:

```sh
bedrock-foundry server expose SERVER_ID --address PUBLIC_IP
bedrock-foundry server run SERVER_ID
```

For the default ports, Foundry writes a NetherNet mapping for public UDP 19133 to
local UDP 19133. The public address must belong to your connection. This command
does not change the router or firewall. Stop the server before changing access.

A connection behind carrier-grade NAT cannot receive ordinary router forwards.
Use a relay, VPN, or tunnel that supports both the required TCP signaling and UDP
transport. Foundry does not provision or manage an external tunnel in this release.
Public exposure and native client joins remain outside the completed live tests.

## Back up a server

Stop the server and select **Back up**. Foundry saves a ZIP in the application's
`backups` directory. It includes worlds, configuration, account allowlists, and
installed packs. The BDS executable and console log are excluded.

To restore a backup, keep the server stopped and extract it into that instance's
directory. Keep a copy of the current instance before replacing its files. If the
backup uses a different BDS version, install the matching official distribution
in a separate instance first. Automatic version upgrades and backup restoration
are not available from the desktop in this release.
