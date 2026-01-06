# RTRTR – An RPKI data proxy

![CI](https://github.com/NLnetLabs/rtrtr/workflows/ci/badge.svg)
[![Documentation Status](https://readthedocs.org/projects/rtrtr/badge/?version=stable)](https://rtrtr.docs.nlnetlabs.nl/en/stable/?badge=stable)
[![crates.io](https://img.shields.io/crates/v/rtrtr.svg?color=brightgreen)](https://crates.io/crates/rtrtr)

[![Discuss on Discourse](https://img.shields.io/badge/Discourse-NLnet_Labs-orange?logo=Discourse)](https://community.nlnetlabs.nl/c/rpki/11)
[![Discord](https://img.shields.io/discord/818584154278199396?label=Discord&logo=discord)](https://discord.gg/8dvKB5Ykhy)
[![Mastodon Follow](https://img.shields.io/mastodon/follow/114692612288811644?domain=social.nlnetlabs.nl&style=social)](https://social.nlnetlabs.nl/@nlnetlabs)

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
server](https://discord.gg/8dvKB5Ykhy). 

## Getting Started

Getting started with RTRTR is really easy by installing a binary package
for either Debian and Ubuntu or for Red Hat Enterprise Linux (RHEL) and
compatible systems such as Rocky Linux. Alternatively, you can run with
Docker or build from the source code using Cargo, Rust’s build system and
package manager.

Please refer to the comprehensive
[documentation](https://rtrtr.docs.nlnetlabs.nl/) to learn what works
best for you.
