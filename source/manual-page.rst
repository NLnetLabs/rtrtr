.. only:: html

    :command:`rtrtr` – RPKI data proxy

    :Date: 2022-01-04
    :Author: Martin Hoffmann
    :Copyright: 2021-2022 – NLnet Labs
    :Version: 0.1.2

    Synopsis
    --------

    .. raw:: html

        <p><strong class="command">rtrtr</strong> <code class="xref std
        std-option docutils literal notranslate"><span
        class="pre">options</span></code></p>

.. only:: man

    Synopsis
    --------

    :command:`rtrtr` [options]

Description
-----------

RTRTR is an RPKI data proxy, designed to collect Validated ROA Payloads
from one or more sources in multiple formats and dispatch it onwards. It 
provides the means to implement multiple distribution architectures for
RPKI such as centralised RPKI validators that dispatch data to local caching
RTR servers.

RTRTR can read RPKI data from multiple RPKI Relying Party packages via RTR
and JSON and, in turn, provide an RTR service for routers to connect to. 
The HTTP server provides the validated data set in JSON format, as well as
a monitoring endpoint in plain text and Prometheus format.

Options
-------

.. option:: -c path, --config=path

    Provides the path to a file containing the configuration for RTRTR. See
    `Configuration File`_ below for more information on the format and
    contents of the file.

    This option is required.

.. option:: -v, --verbose

      Print more information. If given twice, even more information is printed.

      More specifically, a single :option:`-v` increases the log level from the
      default of warn to *info*, specifying it more than once increases it to
      *debug*.
      
      See `Logging`_ below for more information on what information is logged at
      the different levels.

.. option:: -q, --quiet

      Print less information. Given twice, print nothing at all.

      A single :option:`-q` will drop the log level to *error*. Repeating
      :option:`-q` more than once turns logging off completely.

.. option:: --syslog

      Redirect logging output to syslog.

      This option is implied if a command is used that causes Routinator to run
      in daemon mode.

.. option:: --syslog-facility=facility

      If logging to syslog is used, this option can be used to specify the
      syslog facility to use. The default is *daemon*.

.. option:: --logfile=path

      Redirect logging output to the given file.

.. option:: -h, --help

      Print some help information.

.. option:: -V, --version

      Print version information.


Configuration File
------------------

The configuration file describes how and from where RTRTR is collecting data,
how it processes it and how it should provide access to the resulting data
set or data sets.

The configuration file is a file in TOML format. It consists of a
sequence of key-value pairs, each on its own line. Strings are to be enclosed in
double quotes. Lists can be given by enclosing a comma-separated list of values
in square brackets. The file contains multiple sections, each started with a
name enclosed in square brackets.

The first section without a name at the beginning of the file provides
general configuration for RTRTR as a whole. It is followed by a single
section for each component to be started.

There are two types of components: *units* and *targets*. Units take data
from somewhere and produce a single, constantly updated data set. Targets
take the data set from exactly one other unit and serve it in some specific
way.

Both units and targets have a name and a type that defines which particular
kind of unit or target this is. For each type, additional arguments need to
be provided. Which these are and what they mean depends on the type.

The section of a component is named by appending the name of the component to
its class. I.e., a unit named ``foo`` would have a section name of
``[unit.foo]`` while a target ``bar`` would have a section name of
``[target.bar]``.

The following reference lists all configuration options for the global section
as well as all options for each currently defined unit and target type. For
each option it states the name, type, and purpose. Any relative path given as
a configuration value is interpreted relative to the directory the
configuration file is located in.

Global Options
--------------

http-listen
      A list of string values each specifying an address and port the HTTP
      server should listen on. Address and port should be separated by a
      colon. IPv6 address should be enclosed in square brackets.

      RTRTR will listen on all address port combinations specified. All HTTP
      endpoints will be available on all of them.

log-level
      A string value specifying the maximum log level for which log messages
      should be emitted. The default is warn.

log
      A string specifying where to send log messages to. This can be
      one of the following values:

      default
             Log messages will be sent to standard error if Routinator
             stays attached to the terminal or to syslog if it runs in
             daemon mode.

      stderr
             Log messages will be sent to standard error.

      syslog
             Log messages will be sent to syslog.

      file
             Log messages will be sent to the file specified through
             the log-file configuration file entry.

      The default if this value is missing is, unsurprisingly, default.

log-file
      A string value containing the path to a file to which log messages will be
      appended if the log configuration value is set to file. In this case, the
      value is mandatory.

syslog-facility
      A string value specifying the syslog facility to use for logging to
      syslog. The default value if this entry is missing is daemon.


RTR Units
---------

There are two units that download RPKI data sets from an upstream server
using the RPKI-to-Router protocol (RTR). The unit of type ``"rtr"`` uses
unencrypted RTR while ``"rtr-tls"`` uses RTR over TLS.

The RTR units have the following configuration options:

remote
      A string value specifying the remote server to connect to. The string
      must contain both an address and a port separated by a colon. The
      address can be given as a an IP address, enclosed in square brackets
      for IPv6, or a host name.

      For the ``"rtr-tls"`` unit, the address portion will be used to verify
      the server certificate against.

      This option is mandatory.

retry
      An integer value specifying the number of seconds to wait before trying
      to reconnect to the server if it closed the connection.

      If this option is missing, the default of 60 seconds is used.

cacerts
      Only used with the ``"rtr-tls"`` type, a list of paths to files that
      contain one or more PEM encoded certificates that should be trusted when
      verifying a TLS server certificate.

      The ``"rtr-tls"`` unit also uses the usual set of web trust anchors, so
      this option is only necessary when the RTR server doesn’t use a server
      certificate that would be trusted by web browser. This is, for instance,
      the case if the server uses a self-signed certificate in which case this
      certificate needs to be added via this option.


JSON Unit
---------

A unit of type ``"json"`` imports and updates an RPKI data set through a
JSON-encoded file. It accepts the JSON format used by most relying party
packages.

The ``"json"`` unit has the following configuration options:

uri
      A string value specifying the location of the JSON file expressed as a
      URI.

      If this is an ``http:`` or ``https:`` URI, the unit will download the
      file from the given location.

      If this is a ``file:`` URI, the unit will load the given local file.
      Note that the unit just uses the path as given, so relative paths will
      interpreted relative to the current directory, whatever that may be.

refresh
      An integer value specifying the number of seconds to wait before
      attempting to re-fetch the file.

      This value is used independently of whether the previous fetch has
      succeeded or not.

Any Unit
--------

A unit of type ``"any"`` will pick one data set from one of a number of
source units. The unit will only pick a source if it has an updated data set
and can therefore be used to fall back to a different unit if one fails.

The ``"any"`` unit has the following configuration options:

sources
      A list of strings each containing the name of a unit to use as a source.

random
      A boolean value specifying whether the unit should pick a source unit
      at random. If the value is ``false`` or not given, the source units are
      picked in the order given.
 

SLURM Unit
----------

A unit of type ``"slurm"`` will apply local exception rules to a data set
provided by another unit. These rules are defined through local JSON files
as described in :rfc:`8416`. They allow to both filter out existing entries
in a data set as well as add new entries.

The ``"slurm"`` unit has the following configuration options:

source
      A string value specifying the name of the unit that provides the
      data set to apply the local exceptions to.

files
      A list of strings each specifying the path to a local exception file.
      
      The files are continously checked for updates, so RTRTR does not need
      to be restarted if the files are updated.

RTR Targets
-----------

There are two types of targets that provide a data set as an RTR server. The
target of type ``"rtr"`` provides the data set over unencrypted RTR while
the type ``"rtr-tls"`` offers the set through RTR over TLS.

The RTR targets have the following configuration options:

listen
      A list of string values each specifying an address and port the RTR
      target should listen on. Address and port should be separated by a
      colon. IPv6 address should be enclosed in square brackets.

unit
       A string value specifying the name of the unit that provides the data
       set for the RTR target to offer.

The ``"rtr-tls"`` target has the following *additional* configuration options:

certificate
      A string value providing a path to a file containing the PEM-encoded
      certificate to be used as the TLS server certificate.

key
      A string value providing a path to a file containing the PEM-encoded
      certificate to be used as the private key by the TLS server.


HTTP Target
-----------

A target of type ``"http"`` will offer the data set provided by a unit for
download through the HTTP server.

The ``"http"`` target has the following configuration options:

path
      A string value specifying the path in the HTTP server under which the
      target should offer its data.

      All HTTP targets share the same name space in RTRTR’s global HTTP
      server. This value provides the path portion of HTTP URIs. It should
      start with a slash.

format
      A string value specifying the format of the data set to be offered.
      Currently, this has to be ``"json"`` for the JSON format.

unit
       A string value specifying the name of the unit that provides the data
       set for the RTR target to offer.


Logging
-------
In order to allow diagnosis of the operation as well as its overall health,
RTRTR logs an extensive amount of information. The log levels used by
syslog are utilized to allow filtering this information for particular use
cases.

The log levels represent the following information:

error
      Information  related to events that prevent RTRTR from continuing to
      operate at all as well as all issues related to local configuration even
      if RTRTR will continue to run.

warn
      Information  about  events  and  data that influences the data sets
      produced by RTRTR. This includes failures to communicate with
      upstream servers, or encountering invalid data.

info
      Information about events and data that could be considered abnormal but
      do not influence the data set.

debug
      Information about the internal state of RTRTR that may be useful for
      debugging.

