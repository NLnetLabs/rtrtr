# RTRTR – A Versatile Tool for Route Filters

![ci](https://github.com/NLnetLabs/rtrtr/workflows/ci/badge.svg)

RTRTR is a tool that collects, processes, and distributes data for route
filtering. It reads data from various sources, such as validated RPKI
data, IRR data, or local rules, allows selecting, filtering, and otherwise
manipulating this data, and finally feeds it to routers either via
protocols such as RTR or through generated configuration files.

RTRTR is currently in early development. Right now, it can read RPKI data
via RTR from multiple servers and provide it, also via RTR, to routers.
Over time, we will add more functionality.


## Architecture

RTRTR is a very versatile tool. It comes with a number of components for
different purposes that can be connected to serve multiple use cases.
There are two classes of components: _Units_ take filtering data from
somewhere – this could be other units or external sources –, and produce and
constantly update one new set of data. _Targets_ take the data set from
one particular unit and serve it to an external party.

Which components RTRTR will use and how they are connected is described in
a config file. An example can be found in [`etc/rtrtr.conf`]. For the
moment, this example file also serves as a manual for the available
components and their configuration.


## Quick Start

If you have already installed Routinator, this should all be somewhat
familiar.

Assuming you have a newly installed Debian or Ubuntu machine, you will need
to install the C toolchain and Rust. You can then install RTRTR using
Cargo, Rust’s build tool, directly from the repository.

```bash
apt install rsync build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo install  --git https://github.com/NLnetLabs/rtrtr.git
```

If you want to pick up a particular branch, you can do so, too:

```
cargo install  --git https://github.com/NLnetLabs/rtrtr.git --branch foo
```

If you have an older version of Rust and RTRTR, you can update using

```bash
rustup update
cargo install -f --git https://github.com/NLnetLabs/rtrtr.git
```

The `-f` option to `cargo install` overwrites an already installed RTRTR.

Once RTRTR is installed, you need to create a config file that suits your
needs. The example in [`etc/rtrtr.conf`] may be a good way to start. The
config file to use needs to be passed to RTRTR via the `-c` option:

```
rtrtr -c rtrtr.conf
```

[`etc/rtrtr.conf`]: https://github.com/NLnetLabs/rtrtr/blob/main/etc/rtrtr.conf
