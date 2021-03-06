# RTRTR – An RPKI data proxy

![ci](https://github.com/NLnetLabs/rtrtr/workflows/ci/badge.svg)
[![Documentation Status](https://readthedocs.org/projects/rtrtr/badge/?version=stable)](https://rtrtr.docs.nlnetlabs.nl/en/stable/?badge=stable)
[![](https://img.shields.io/crates/v/rtrtr.svg?color=brightgreen)](https://crates.io/crates/rtrtr)
[![](https://img.shields.io/discord/818584154278199396?label=rpki%20on%20discord&logo=discord)](https://discord.gg/8dvKB5Ykhy)

RTRTR is an RPKI data proxy, designed to collect Validated ROA Payloads
from one or more sources in multiple formats and dispatch it onwards. It 
provides the means to implement multiple distribution architectures for
RPKI such as centralised RPKI validators that dispatch data to local caching
RTR servers.

RTRTR can read RPKI data from multiple RPKI Relying Party packages via RTR
and JSON and, in turn, provide an RTR service for routers to connect to. 
The HTTP server provides the validated data set in JSON format, as well as
a monitoring endpoint in plain text and Prometheus format.

If you have feedback, we would love to hear from you. Don’t hesitate to [create
an issue on Github](https://github.com/NLnetLabs/rtrtr/issues/new) or post
a message on our [RPKI mailing
list](https://lists.nlnetlabs.nl/mailman/listinfo/rpki) or [Discord
server](https://discord.gg/8dvKB5Ykhy). You can learn more by reading the 
[RTRTR documentation](https://rtrtr.docs.nlnetlabs.nl/) and the
[RPKI technology documentation](https://rpki.readthedocs.io/) on Read the Docs.

## Architecture

RTRTR is a very versatile tool. It comes with a number of components for
different purposes that can be connected to serve multiple use cases.
There are two classes of components: _Units_ take filtering data from
somewhere – this could be other units or external sources –, and produce and
constantly update one new set of data. _Targets_ take the data set from
one particular unit and serve it to an external party.

Which components RTRTR will use and how they are connected is described in
[the documentation](https://rtrtr.docs.nlnetlabs.nl/) Also, an example 
config file can be found in [`etc/rtrtr.conf`].

## Quick Start with Binary Packages

On the NLnet Labs software package repository we provide RTRTR packages for
amd64/x86_64 architectures running Debian and Ubuntu, as well as Red Hat 
Enterprise Linux and CentOS.

### Installing on Debian/Unbuntu

Add the line below that corresponds to your operating system to your
`/etc/apt/sources.list` or `/etc/apt/sources.list.d/`

```bash
deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ stretch main
deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ buster main
deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ xenial main
deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ bionic main
deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ focal main
```

Then run the following commands to add the public key and update the repository 
list

```bash
wget -qO- https://packages.nlnetlabs.nl/aptkey.asc | sudo apt-key add -
sudo apt update
```

You can then install RTRTR by running this command

```bash
sudo apt install rtrtr
```

### Installing on RHEL/CentOS

Create a file named `/etc/yum.repos.d/nlnetlabs.repo`, enter this configuration
and save it:

```bash
[nlnetlabs]
name=NLnet Labs
baseurl=https://packages.nlnetlabs.nl/linux/centos/$releasever/main/$basearch
enabled=1
```
Then run the following command to add the public key

```bash
sudo rpm --import https://packages.nlnetlabs.nl/aptkey.asc
```

You can then install RTRTR by running this command

```bash
sudo yum install -y rtrtr
```

### Setting up RTRTR

You can now configure RTRTR by editing `/etc/rtrtr.conf` and start it with
`sudo systemctl enable --now rtrtr`. You can check the status with the 
command `sudo systemctl status rtrtr` and view the logs with 
`sudo journalctl --unit=rtrtr`.

## Quick Start with Cargo

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

## Quick Start with Docker

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
