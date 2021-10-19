.. _doc_rtrtr_example:

Example Scenario
================

.. figure:: img/rtrtr-flow-example.*
    :align: center
    :width: 100%
    :alt: Example of an RTRTR data flow

    Example of an RTRTR data flow



.. code-block:: text

    # The minimum log level to consider.
    log_level = "debug"

    # The target for logging. This can be "syslog", "stderr", "file", or "default".
    log_target = "stderr"

    # If syslog is used, the syslog facility can be given.
    log_facility = "daemon"

    # If file logging is used, the log file must be given.
    log_file = "/var/log/rtrtr.log"

The built-in HTTP server provides status information at the :command:`/status`
path and Prometheus metrics at the :command:`/metrics` path:

.. code-block:: text

    http-listen = ["127.0.0.1:8080"]

Units and Targets
-----------------

RTRTR has four types of units and two types of targets. Each unit and target
gets its own section in the config. The name of the section, given in square
brackets, describes whether a unit or target is wanted and, after a dot, the
name of the unit or target.

RTR Unit
++++++++

The RTR unit takes a feed of Validated ROA Payloads (VRPs) from a Relying Party
software instance via the RTR protocol. In the example we'll call the unit
``eu-west-3`` because that is the location the validator instance in running in.
The type is ``rtr`` and the final argument is the IP or hostname of the
instance to connect to, along with the port.

Note that this unit does not require a ``refresh`` option, as the RTR protocol


.. code-block:: text

    [units.eu-west-3]
    type = "rtr"
    remote = "paris.validator.example.net:3323"

JSON Unit
+++++++++

Most Relying Party software packages can produce the Validated ROA Payload set
in JSON format as well, either as a file on disk or at an HTTP endpoint. RTRTR
can use this format as a data source too, using the JSON Unit. In the example
we'll use the public JSON feed provided by Cloudflare.

.. code-block:: text


    [units.cloudflare-json]
    type = "json"
    uri = "https://rpki.cloudflare.com/rpki.json"
    refresh = 60

Any Unit
++++++++

The second unit type is called ``any``. It is given any number of other units
and picks the data set from one of them. Units can signal that they currently
don’t have an up-to-date dataset available, so an any unit can skip those and
make sure to always have an up-to-date data set.

.. code-block:: text

    [units.any-rtr]
    type = "any"
    sources = [ "local-3323", "local-3324", "cloudflare-json" ]
    random = false

RTR Target
++++++++++

Finally, we need to do something with the data: serve it via RTR. This is what 
the ``rtr`` target does:

.. code-block:: text

    [targets.local-9001]
    type = "rtr"
    listen = [ "127.0.0.1:9001" ]
    unit = "any-rtr"

JSON Target
+++++++++++

Wee something JSON Target!

.. code-block:: text

    [targets.http-json]
    type = "http"
    path = "/json"
    format = "json"
    unit = "any-rtr"
    
