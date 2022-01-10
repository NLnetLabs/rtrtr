# Change Log

## Unreleased next version

Breaking Changes

* The minimum supported Rust version is now 1.54. ([#45])

New

* The `"json"` unit now supports the modified JSON format used by newer
  versions of rpki-client. That is, it accepts ASNs as numbers or
  strings and ignores any fields that aren’t essential. ([#30], [#32])
* Added a `"slurm"` unit that can be used to manipulate payload sets based
  on local exception files defined in [RFC 8416]. ([#31])
* Added `"rtr-tls"` unit and target that send RTR data over TLS
  connections. ([#34])
* New metrics for the `"rtr"` and `"rtr-tls"` units list the session ID,
  serial number, and time of the last update, as well as total number of
  bytes read from and sent to the server. ([#40])

Bug Fixes

* Corrected the RTR PDU type of the Cache Reset PDU from 7 to 8.
  ([rpki #151])
* The `--config` command line option is now mandatory, resulting in a
  proper error message when it is missing rather than a panic. ([#41])

Other

* Upgraded to Tokio 1.0, Hyper 0.14, and Reqwest 0.11. ([#17]) 

[#30]: https://github.com/NLnetLabs/rtrtr/pull/30
[#31]: https://github.com/NLnetLabs/rtrtr/pull/31
[#32]: https://github.com/NLnetLabs/rtrtr/pull/32
[#34]: https://github.com/NLnetLabs/rtrtr/pull/34
[#40]: https://github.com/NLnetLabs/rtrtr/pull/40
[#41]: https://github.com/NLnetLabs/rtrtr/pull/41
[#45]: https://github.com/NLnetLabs/rtrtr/pull/45
[rpki #151]: https://github.com/NLnetLabs/rpki-rs/pull/151
[RFC 8416]: https://tools.ietf.org/html/rfc8416


## 0.1.2 ‘Ten Four’

Released 2021-03-15

New

* The JSON unit ignores the `metadata` field in received files. This
  makes it compatible with the JSON produced by at least Routinator, OctoRPKI,
  and rpki-client. ([#8])


[#8]: https://github.com/NLnetLabs/rtrtr/pull/8


## 0.1.1 ‘Death Metal Karaoke’

Released 2020-12-11

New

* Support for JSON via HTTP and from a local file as a source, and JSON
  via HTTP as a target. ([#5])

[#5]: https://github.com/NLnetLabs/rtrtr/pull/5


## 0.1.0 ‘Little Ball of Fur’

Released 2020-11-09.

Initial public release.

