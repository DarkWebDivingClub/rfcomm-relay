# rfcomm-relay

A bridge between Bluetooth RFCOMM and a Nostr relay. It listens on an
RFCOMM channel, accepts a connection from the KeyMaster Android app, and
forwards the JSONL-framed Nostr traffic to a local relay over a
WebSocket — in both directions, for as long as the link lasts.

This is the transport the phone uses to reach a desktop Avatar. There is
no IP layer involved: no DHCP, no Wi-Fi, no captive-portal policy, and
no Bluetooth PAN. Paired devices open a serial stream and talk.

```
 KeyMaster Android            this daemon              strfry
   (phone)                                             :7777
      |                                                  |
      |  RFCOMM, Bluetooth Classic, channel 3            |
      +------------------------------------------>  +----+----+
                                                    | bridge  |
                                          WebSocket |         |
                                                    +---------+
```

`km-avatar` is a WebSocket client of the same relay, so the phone and
the avatar meet at `strfry` rather than talking directly.

## Why RFCOMM

Bluetooth PAN (`bt-nap`) came first and was a recurring source of
problems: Android blocks unicast traffic on networks it considers
unvalidated, sleep/wake leaves stale BNEP sessions, and the whole thing
needed a bridge, a DHCP server, a udev rename guard and a sleep hook to
stay upright. RFCOMM is a raw serial stream — reconnection after sleep
is handled by the Bluetooth stack itself. See keymaster mission 36.

## Install

```sh
sudo apt install rfcomm-relay
```

It is a dependency of `keymaster-avatar`, so installing the avatar pulls
it in. The packaged unit runs:

```
/usr/bin/rfcomm-relay --relay ws://localhost:7777
```

## Usage

| Flag | Default | Meaning |
|---|---|---|
| `--relay <url>` | `ws://localhost:7777` | WebSocket relay to bridge to |
| `--channel <n>` | `3` | RFCOMM channel to listen on; `0` auto-assigns |
| `--echo` | off | Reflect received data instead of bridging — for testing the link without a relay |

The channel must match the `rfcomm_channel` the avatar advertises in
`/run/keymaster-avatar/descriptor.json`, which is where the phone reads
it from. Both default to 3.

## Build

Requires a Rust toolchain and `libdbus`/BlueZ headers; it talks to
`bluetoothd` through [`bluer`](https://crates.io/crates/bluer).

```sh
cargo build --release
```

Debian packaging lives on the `packaging/ubuntu/resolute` branch, not on
`master`.

## Operating notes

- **It needs `bluetoothd` running**, and the phone must already be
  paired. The unit declares `Requires=bluetooth.service`.
- **Restarting it drops the phone's link**, which makes the phone dial
  back within seconds and re-attach. That is the standard way to recover
  a session that has gone one-sided — for example after `km-avatar` has
  been restarted on its own, which leaves the phone believing it is
  still attached.
- Logs every frame at `RUST_LOG=info` as `RFCOMM -> WS` and
  `WS -> RFCOMM`, prefixed with the phone's Bluetooth address, which
  makes `journalctl -u rfcomm-relay` the quickest way to see whether the
  phone is actually talking.

## License

GNU General Public License v3.0 or later (`GPL-3.0-or-later`), as
declared in `Cargo.toml`.
