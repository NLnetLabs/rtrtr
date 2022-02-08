RTRTR |version|
===============

.. only:: html

    |lastcommit| |license| |discord|

    .. |lastcommit| image:: https://img.shields.io/github/last-commit/nlnetlabs/rtrtr
                :target: https://github.com/NLnetLabs/rtrtr

    .. |license| image:: https://img.shields.io/github/license/nlnetlabs/rtrtr
                :target: https://github.com/NLnetLabs/rtrtr/blob/main/LICENSE

    .. |discord| image:: https://img.shields.io/discord/818584154278199396?label=rpki%20on%20discord&logo=discord
                :target: https://discord.gg/8dvKB5Ykhy

A versatile toolbox
   RTRTR is an RPKI data proxy, designed to collect Validated ROA Payloads
   from one or more sources in multiple formats and dispatch it onwards. It
   provides the means to implement multiple distribution architectures for
   RPKI such as centralised RPKI validators that dispatch data to local
   caching RTR servers.

Secure and redundant RTR connections
   RTRTR can read RPKI data from multiple RPKI Relying Party packages via RTR
   and JSON and, in turn, provide an RTR service for routers to connect to.
   The HTTP server provides the validated data set in JSON format, as well as
   a monitoring endpoint in plain text and Prometheus format. TLS is
   supported on all connections.

Open source with community and professional support
   NLnet Labs offers `professional support services
   <https://www.nlnetlabs.nl/services/contracts/>`_ with a service-level
   agreement. We also provide a `mailing list
   <https://lists.nlnetlabs.nl/mailman/listinfo/rpki>`_ and `Discord server
   <https://discord.gg/8dvKB5Ykhy>`_  for community support and to exchange
   operational experiences. RTRTR is liberally licensed under the `BSD
   3-Clause license
   <https://github.com/NLnetLabs/rtrtr/blob/main/LICENSE>`_.

.. toctree::
   :maxdepth: 2
   :hidden:

   installation
   configuration
   example-scenario
   manual-page

.. history
.. authors
.. license
