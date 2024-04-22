Configuration
=============

RTRTR uses two classes of components: *units* and *targets*. Units take data
from somewhere and produce a single, constantly updated data set. Targets take
the data set from exactly one other unit and serve it in some specific way.

Both units and targets have a name — so that we can refer to them — and a type
that defines which particular kind of unit or target this is. For each type,
additional arguments need to be provided. Which these are and what they mean
depends on the type.

Units and targets can be wired together in any way to achieve your specific
goal. This is done in a configuration file, which also specifies several general
parameters for logging, as well as status and Prometheus metrics endpoints via
the built-in HTTP server.

.. Note:: The configuration file is in :abbr:`TOML (Tom's Obvious Minimal 
          Language)` format, which is somewhat similar to INI files. You can 
          find more information on the `TOML website <https://toml.io/en/>`_. 

General Parameters
------------------

The configuration file starts out with a number of optional parameters to
specify logging. The built-in HTTP server provides status information at the
:command:`/status` path and Prometheus metrics at the :command:`/metrics` path.
Note that details are provided for each unit and each target.

.. code-block:: text

    # The minimum log level to consider.
    log_level = "debug"

    # The target for logging. This can be "syslog", "stderr", "file", or "default".
    log_target = "stderr"

    # If syslog is used, the syslog facility can be given.
    log_facility = "daemon"

    # If file logging is used, the log file must be given.
    log_file = "/var/log/rtrtr.log"

    # Where should the HTTP server listen on?
    http-listen = ["127.0.0.1:8080"]

    # The proxy servers to use for outgoing HTTP requests.
    #
    # Note: This option is only used if RTRTR is built with the socks feature
    # enabled. This is true by default.
    http-proxies = [ "socks5://192.168.1.3:9000" ]

    # Additional root certificates for outgoing HTTP requests
    http-root-certs = [ "/var/lib/rtrtr/root-cert/some.crt" ]

    # The user agent string to use for outgoing HTTP requests.
    http-user-agent = "My RPKI proxy"

    # Local address to bind to for outgoing HTTP requests.
    http-client-addr = "198.168.1.2"

Units
-----

RTRTR currently has four types of units. Each unit gets its own section in the
configuration. The name of the section, given in square brackets, starts with
``units.`` and is followed by a descriptive name you set, which you can later
refer to from other units, or a target.

RTR Unit
++++++++

The unit of the type ``rtr`` takes a feed of Validated ROA Payloads (VRPs) from
a Relying Party software instance via the RTR protocol. Along with a unique
name, the only required argument is the IP or hostname of the instance to
connect to, along with the port. 

Because the RTR protocol uses sessions and state, we don't need to specify a
refresh interval for this unit. Should the server close the connection, by
default RTRTR will retry every 60 seconds. This value is configurable wih the
:option:`retry` option.

.. code-block:: text

    [units.rtr-unit-name]
    type = "rtr"
    remote = "validator.example.net:3323"

It's also possible to configure RTR over TLS, using the ``rtr-tls`` unit type.
When using this unit type, there is an additional configuration option,
:option:`cacerts`, which specifies a list of paths to files that contain one or
more PEM encoded certificates that should be trusted when verifying a TLS server
certificate.

The ``rtr-tls`` unit also uses the usual set of web trust anchors, so this
option is only necessary when the RTR server doesn’t use a server certificate
that would be trusted by web browser. This is, for instance, the case if the
server uses a self-signed certificate in which case this certificate needs to be
added via this option.

JSON Unit
+++++++++

Most Relying Party software packages can produce the Validated ROA Payload set
in JSON format as well, either as a file on disk or at an HTTP endpoint. RTRTR
can use this format as a data source too, using units of the type ``json``. 
Along with specifying a name, you must specify the URI to fetch the VRP set
from, as well as the refresh interval in seconds.

.. code-block:: text

    [units.json-unit-name]
    type = "json"
    uri = "http://validator.example.net/vrps.json"
    refresh = 60

Any Unit
++++++++

The ``any`` unit type is given any number of *other* units and picks the data
set from one of them. Units can signal that they currently don’t have an
up-to-date data set available, allowing the ``any`` unit to skip those. This
ensures there is always an up-to-date data set available.

.. Important:: The ``any`` unit uses a single data source at a time. RTRTR does 
               **not** attempt to make a union or intersection of multiple VRPs
               sets, to avoid the risk of making a route *invalid* that would
               otherwise be *unknown*.

To configure this unit, specify a name, set the type to ``any`` and list the
sources that should be used. Lastly, specify if a random unit should be selected
every time it needs to switch or whether it should go through the list in order.

.. code-block:: text

    [units.any-unit-name]
    type = "any"
    sources = [ "unit-1", "unit-2", "unit-3" ]
    random = false

SLURM Unit
++++++++++

In some cases, you may want to override the global RPKI data set with your own
local exceptions. You can do this by specifying route origins that should be
filtered out of the output, as well as origins that should be added, in a file
using JSON notation according to the :abbr:`SLURM (Simplified Local Internet
Number Resource Management with the RPKI)` standard specified in :RFC:`8416`.

You can refer to the JSON file you created with a unit of the type ``slurm``. As
the source to which the exceptions should be applied, you must specify any of
the other units you have created. Note that the :option:`files` attribute is an
array and can take multiple values as input.

.. code-block:: text

    [units.slurm]
    type = "slurm"
    source = "source-unit-name"
    files = [ "/var/lib/rtrtr/local-expections.json" ]

The :doc:`routinator:local-exceptions` page in the Routinator documentation
has more information on the format and syntax of SLURM files. 

Targets
-------

RTRTR currently has two types of targets. As with units, each unit gets its own
section in the configuration. And also here, the name of the section starts with
``targets.`` and is followed by a descriptive name you set, all enclosed in
square brackets.

RTR Target
++++++++++

Targets of the type ``rtr`` let you serve the data you collected with your units
via the RPKI-to-Router (RTR) protocol. You must give your target a name and
specify the host name or IP address it should listen on, along with the port. As
the RTR target can listen on  multiple addresses, the listen argument is a list.
Lastly, you must specify the name of the unit the target should receive its data
from.

.. code-block:: text

    [targets.rtr-target-name]
    type = "rtr"
    listen = [ "127.0.0.1:9001" ]
    unit = "source-unit-name"

The three optional configuration options ``refresh``, ``retry`` and ``expire``
allow setting the respective fields in the timer values sent to the client.
If they are missing, the default values are used.

This target also supports TLS connections, via the ``rtr-tls`` type. This target
has two additional configuration options. First, the :option:`certificate`
option, which is a string value providing a path to a file containing the
PEM-encoded certificate to be used as the TLS server certificate. And secondly,
there is the :option:`key` option, which provides a path to a file containing
the PEM-encoded certificate to be used as the private key by the TLS server.

HTTP Target
+++++++++++

Targets of the type ``http`` let you serve the collected data via HTTP, which is
currently only possible in ``json`` format. You can us this data stream for
monitoring, provisioning, your IP address management, or any other purpose that
you require. To use this target, specify a name and a path, as well as the name
of the unit the target should receive its data from.

.. code-block:: text

    [targets.http-target-name]
    type = "http"
    path = "/json"
    format = "json"
    unit = "source-unit-name"
    
