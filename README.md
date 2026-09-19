# agent-slint

**Easy Agent** — desktop client in Rust using [Slint](https://slint.dev),
over the same easy-rpc (Connect) wire as every other Easy Agent client.

It consumes the generated [`agent-sdk-rust`](https://github.com/easy-utils/agent-sdk-rust)
messages and the [`easy-rpc-rust`](https://github.com/easy-utils/easy-rpc-rust)
transport; the app owns only the RPC facade and the UI.

```bash
make run            # Slint needs X11/GL/EGL dev libs
```

Connect with the standalone agent's base URL + a tenant token, list sessions,
open a chat, and stream a prompt turn (`text-delta`, `reasoning-delta`,
`tool-call`).
