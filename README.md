# RTRTR – The RPKI Express Mail Service


RTRTR is a companion tool to
[Routinator][https://github.com/NLnetLabs/routinator] that collects,
processes, and serves validated RPKI data from multiple sources. The
source data can be provided in different formats and the produced data can
be provided in different formats.

This is the very first iteration of RTRTR, however. For now, it only
operates a simple RTR proxy: It collects validated RPKI data from
Routinator or
[some other RPKI relying party
software][https://rpki.readthedocs.io/en/latest/tools.html#relying-party-software] via RTR and serves this data via RTR. This way, the data provided
by a centralized relying party software can be distributed to RTR servers
in multiple location, allowing routers to only connect to local servers.

Over time, RTRTR will gain more and more capabilities. Stay tuned!


## Quick Start

If you have already installed Routinator, this should all be somewhat
familiar.

Assuming you have a newly installed Debian or Ubuntu machine, you will need
to install the C toolchain and Rust. You can then install RTRTR and start
it up serving RTR listening on 127.0.0.1 port 3323 and collecting data
from two upstream RTR caches on 127.0.0.1 port 3324 and 127.0.0.1 port 3325.
Note that RTRTR will use the first server as long as it is available and
will only fall back to the second one if the first one fails.

```bash
apt install rsync build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo install https://github.com/NLnetLabs/rtrtr.git
rtrtr --rtr-listen 127.0.0.1:3323 \
  --rtr-server 127.0.0.1:3324 --rtr-server 127.0.0.1:3325
```

If you have an older version of Rust and RTRTR, you can update using

```bash
rustup update
cargo install -f https://github.com/NLnetLabs/rtrtr.git
```


