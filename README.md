# RTRTR – An RPKI data proxy

![ci](https://github.com/NLnetLabs/rtrtr/workflows/ci/badge.svg)
[![](https://img.shields.io/crates/v/rtrtr.svg?color=brightgreen)](https://crates.io/crates/rtrtr)

RTRTR is currently in early development. Right now, it can read RPKI data
from multiple RPKI Relying Party packages via RTR and provide it, also via
RTR, to routers. The HTTP server provides a monitoring endpoint in plain
text and Prometheus format.

Over time, we will add more functionality, such as transport using RTR 
over TLS, as well as plain and signed JSON over HTTPS.

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
Cargo, Rust’s build tool.

```bash
apt install build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo install --locked rtrtr
```
If you have an older version of Rust and RTRTR, you can update using

```bash
rustup update
cargo install --locked --force rtrtr
```
If you want to try the main branch from the repository instead of a
release version, you can run

```bash
cargo install --git https://github.com/NLnetLabs/rtrtr.git --branch main
```

Once RTRTR is installed, you need to create a config file that suits your
needs. The example in [`etc/rtrtr.conf`] may be a good way to start. The
config file to use needs to be passed to RTRTR via the `-c` option:

```
rtrtr -c rtrtr.conf
```

[`etc/rtrtr.conf`]: https://github.com/NLnetLabs/rtrtr/blob/main/etc/rtrtr.conf

## Using Docker

To run RTRTR with Docker you will first need to create an `rtrtr.conf` file
somewhere on your host computer and make that available to the Docker container
when you run it. For example if your config file is in `/etc/rtrtr.conf` on the
host computer:

```bash
docker run -v /etc/rtrtr.conf:/etc/rtrtr.conf nlnetlabs/rtrtr -c /etc/rtrtr.conf
```

RTRTR will need network access to fetch and publish data according to the
configured units and targets respectively. Explaining Docker networking is beyond
the scope of this README, however below are a couple of examples to get you
started.

If you need an RTRTR unit to fetch data from a source port on the host you will
also need to give the Docker container access to the host network. For example
one way to do this is with `--net=host`:

```bash
docker run --net=host ...
```
_(where ... represents the rest of the arguments to pass to Docker and RTRTR)_

This will also cause any configured RTRTR target ports to be published on the
host network interface.

If you're not using `--net=host` you will need to tell Docker to expoee the
RTRTR target ports, either one by one using `-p`, or you can publish the default
ports exposed by the Docker container (and at the same time remap them to high
numbered ports) using `-P`. E.g.

```bash
docker run -p 8080:8080/tcp -p 9001:9001/tcp ...
```

Or:

```bash
docker run -P ...
```

You can verify which ports are exposed using the `docker ps` command which should
show something like this:
```bash
CONTAINER ID   IMAGE             COMMAND                  CREATED          STATUS          PORTS                                              NAMES
146237ba9b4b   nlnetlabs/rtrtr   "/sbin/tini -- rtrtr…"   16 seconds ago   Up 14 seconds   0.0.0.0:49154->8080/tcp, 0.0.0.0:49153->9001/tcp   zealous_tesla
```
_(the output in this example shows the high-numbered port mapping that occurs when using `docker run -P`)_
