# Deploying the test server

This puts the server on a VPS behind HTTPS, with the fake phone looping a test
clip so the viewer page always has a stream to show. HTTPS isn't optional: the
phone's camera and the viewers' WebCodecs decoder only work on HTTPS pages.

## What you need

- A small Linux VPS with Ubuntu 24.04 and a public IPv4 address. 1 vCPU and 2
  GB of RAM is plenty; pick a data centre near Romania for the lowest delay.
- A domain name (or a free subdomain, e.g. from DuckDNS) with an A record
  pointing at the VPS.
- Bandwidth: every viewer receives about 1.8 Mbit/s. A 30-minute session with
  20 viewers sends about 8 GB, well inside what VPS plans include.

## Set it up

As root on the VPS:

```sh
# Tools, Rust, and the code
apt update && apt install -y build-essential git ffmpeg curl caddy
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
git clone https://github.com/Exly78/Larpity.git /opt/larpity
cd /opt/larpity/server
cargo build --release
scripts/make-test-clip.sh /opt/larpity/test-clip.h264

# Settings, with a fresh random token
cp deploy/larpity.env.example /etc/larpity.env
sed -i "s/^LARPITY_TOKEN=.*/LARPITY_TOKEN=$(openssl rand -hex 16)/" /etc/larpity.env
chmod 600 /etc/larpity.env

# The server, the fake phone and the stub AI host
cp deploy/*.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now larpity larpity-fake-phone larpity-stub-ai

# HTTPS (replace the domain)
sed "s/larpity.example.com/larpity.yourdomain.ro/" deploy/Caddyfile > /etc/caddy/Caddyfile
systemctl reload caddy

# Firewall
ufw allow OpenSSH && ufw allow 80,443/tcp && ufw --force enable
```

## Share the links

The token is in `/etc/larpity.env`.

| Who | Link |
|---|---|
| Viewers | `https://larpity.yourdomain.ro/` |
| The phone | `https://larpity.yourdomain.ro/phone/?token=TOKEN` |
| The AI host | `wss://larpity.yourdomain.ro/ws/ai?token=TOKEN` |

Check it is up with `curl https://larpity.yourdomain.ro/health`, which returns `ok`.

## Day to day

```sh
journalctl -u larpity -f                       # server logs
systemctl start larpity-fake-phone             # after a real phone has finished
systemctl disable --now larpity-stub-ai        # once the real AI host connects

# Update to the latest code
cd /opt/larpity && git pull && cd server && cargo build --release
systemctl restart larpity larpity-fake-phone larpity-stub-ai
```

When the real phone connects, the fake phone hands over and stops. The same
goes for the stub AI host when the real one connects. For a real session, turn
recording on by uncommenting `LARPITY_RECORD_DIR` in `/etc/larpity.env` and
restarting `larpity`.
