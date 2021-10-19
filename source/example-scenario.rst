.. _doc_rtrtr_example:

Example Scenario
================

To make it clearer how you can deploy RTRTR, below is an example scenario. This
flow may not be entirely realistic, but it intends to show all the different
ways you can wire units and targets together using a visual representation and
the configuration file needed to accomplish it.

In this example, there is routing infrastructure in a European data centre
(DC1). To ensure redundancy, it gets Validated ROA Payloads (VRPs) primarily
from Relying Party software running in the ``eu-west-3`` data centre, using the
RTR protocol. There are two backups configured: another validator serving RTR,
running in ``ap-south-1`` and, if that fails as well, the public JSON feed
Cloudflare offers as a last resort. A unit of the type ``any`` is configured to
get a feed from all three and, should the first one fail, do a round robin to
the next available one.

To make the management of some statically configured routes for this location 
easy, the ``slurm`` unit gets its data from the ``any`` unit so only a single
file has to be kept up-to-date.

Finally, an ``http`` target is configured to get the VRPs without the SLURM 
exceptions, to be fed into internal tooling running in the headquarters. 
Finally, an ``rtr`` unit is defined to serve the routing infrastructure.

.. figure:: img/rtrtr-flow-example.*
    :align: center
    :width: 100%
    :alt: Example of an RTRTR data flow

    Example of an RTRTR data flow

Configuration File
------------------

.. code-block:: text

    log_level = "debug"
    log_target = "stderr"
    log_facility = "daemon"
    log_file = "/var/log/rtrtr.log"

    http-listen = ["eu-dc1.http.example.net:8080"]

    # RTR UNITS

    [units.eu-west-3]
    type = "rtr"
    remote = "paris.validator.example.net:3323"

    [units.ap-south-1]
    type = "rtr"
    remote = "mumbai.validator.example.net:3323"

    # JSON UNIT 

    [units.cloudflare-json]
    type = "json"
    uri = "https://rpki.cloudflare.com/rpki.json"
    refresh = 60

    # ANY UNIT

    [units.round-robin]
    type = "any"
    sources = [ "eu-west-3", "ap-south-1", "cloudflare-json" ]
    random = false

    # SLURM

    [units.static-routes]
    type = "slurm"
    source = "round-robin"
    files = [ "/var/lib/rtrtr/local-expections.json" ]

    # RTR TARGET

    [targets.eu-dc1]
    type = "rtr"
    listen = [ "eu-dc1.rtr.example.net:9001" ]
    unit = "static-routes"

    # JSON TARGET

    [targets.hq-ipam]
    type = "http"
    path = "/json"
    format = "json"
    unit = "round-robin"