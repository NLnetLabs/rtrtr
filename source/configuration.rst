.. _doc_rtrtr_configuration:

Configuration
=============

A configuration file is required for RTRTR to run. It describes which components
should be loaded and how they will be connected. The file is in a format call
:abbr:`TOML (Tom's Obvious Minimal Language)`, which is somewhat similar to INI
files. You can find more information on the `TOML website
<https://toml.io/en/>`_. 

The file’s content starts out with a number of optional general parameters:

.. code-block:: text

    # The minimum log level to consider.
    log_level = "debug"

    # The target for logging. This can be "syslog", "stderr", "file", or
    # "default".
    log_target = "stderr"

    # If syslog is used, the syslog facility can be given:
    log_facility = "daemon"

    # If file logging is used, the log file must be given.
    log_file = "/var/log/rtrtr.log"

RTRTR has a built in HTTP server that provides status information at the 
:command:`/status` path and Prometheus metrics at the :command:`/metrics` path:

.. code-block:: text

    # Where should the HTTP server listen on?
    #
    # The HTTP server provides access to Prometheus-style metrics under the
    # `/metrics` path and plain text status information under `/status` and
    # can be used as a target for serving data (see below for more on targets).
    http-listen = ["127.0.0.1:8080"]

RTRTR uses two classes of components: *units* and *targets*. Units take data
from somewhere and produce a single, constantly updated data set. Targets take
the data set from exactly one other unit and serve it in some specific way.

Both units and targets have a name — so that we can refer to them — and a type
that defines which particular kind of unit or target this is. For each type,
additional arguments need to be provided. Which these are and what they mean
depends on the type.

At this time, there are only two types of units and one type of target. Each
unit and target gets its own section in the config. The name of the section,
given in square brackets, describes whether a unit or target is wanted and,
after a dot, the name of the unit or target.

Let's start with a unit for an RTR client. We call it ``local-3323`` because
it connects to port 3323 on localhost. You can, of course, choose whatever
name you like:

.. code-block:: text

    [units.local-3323]

The type of this unit is ``rtr`` for an RTR client using plain TCP:

.. code-block:: text

    type = "rtr"

The rtr unit needs one more argument: where to connect to:

.. code-block:: text

    remote = "localhost:3323"

Let’s add another RTR unit for another server:

.. code-block:: text

    [units.local-3324]
    type = "rtr"
    remote = "localhost:3324"

    [units.local-json]
    type = "json"
    uri = "http://localhost:8323/json"
    refresh = 60

    [units.cloudflare-json]
    type = "json"
    uri = "https://rpki.cloudflare.com/rpki.json"
    refresh = 60

The second unit type is called ``any``. It is given any number of other units
and picks the data set from one of them. Units can signal that they currently
don’t have an up-to-date dataset available, so an any unit can skip those and
make sure to always have an up-to-date data set.

.. code-block:: text

    [units.any-rtr]
    type = "any"

The names of the units the any unit should get its data from:

.. code-block:: text

    sources = [ "local-3323", "local-3324", "cloudflare-json" ]

Whether the unit should pick a unit random every time it needs to switch
or rather go through the list in order:

.. code-block:: text

    random = false

Finally, we need to do something with the data: serve it via RTR. This is what 
the ``rtr`` target does:

.. code-block:: text

    [targets.local-9001]
    type = "rtr"

The ``rtr`` target can listen on multiple addresses, so the listen argument is a 
list:

.. code-block:: text

    listen = [ "127.0.0.1:9001" ]

The name of the unit the target should receive its data from:

.. code-block:: text

    unit = "any-rtr"

.. code-block:: text

    [targets.http-json]
    type = "http"
    path = "/json"
    format = "json"
    unit = "any-rtr"
