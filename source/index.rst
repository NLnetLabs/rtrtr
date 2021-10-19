.. _doc_rtrtr:

RTRTR – *main* branch
=====================

.. only:: html

    |docsupdated| |discord|

    .. |docsupdated| image:: https://img.shields.io/github/last-commit/NLnetLabs/rtrtr-manual.svg?label=docs%20updated
                :target: https://github.com/NLnetLabs/routinator-manual/commits/main

    .. |discord| image:: https://img.shields.io/discord/818584154278199396?label=rpki%20on%20discord&logo=discord
                :target: https://discord.gg/8dvKB5Ykhy

RTRTR is an RPKI data proxy, designed to collect Validated ROA Payloads from one
or more sources in multiple formats and dispatch it onwards. It provides the
means to implement multiple distribution architectures for RPKI such as
centralised RPKI validators that dispatch data to local caching RTR servers.

RTRTR can read RPKI data from multiple RPKI Relying Party packages via RTR and
JSON and, in turn, provide an RTR service for routers to connect to. The HTTP
server provides the validated data set in JSON format, as well as a monitoring
endpoint in plain text and Prometheus format.

If you run into a problem with RTRTR or you have a feature request, please
`create an issue on Github <https://github.com/NLnetLabs/rtrtr/issues>`_.
We are also happy to accept your pull requests. For general discussion and
exchanging operational experiences we provide a `mailing list
<https://lists.nlnetlabs.nl/mailman/listinfo/rpki>`_ and a `Discord server
<https://discord.gg/8dvKB5Ykhy>`_. 

.. toctree::
   :maxdepth: 2

   introduction
   installation
   configuration
   example-scenario
   
.. history
.. authors
.. license
