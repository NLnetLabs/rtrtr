.. _doc_rtrtr_installation:

Installation
============

System Requirements
-------------------

When choosing a system to run RTRTR on, make sure you have 1GB of available
memory and 1GB of disk space. 

Quick Start
-----------

Getting started with RTRTR is really easy by either installing a binary package
for Debian and Ubuntu or for Red Hat Enterprise Linux and CentOS. You can also
run with Docker or build from Cargo, Rust's build system and package manager.

.. tabs::

   .. group-tab:: Deb Packages

       If you have a machine with an amd64/x86_64 architecture running a recent
       Debian or Ubuntu distribution, you can install RTRTR from our `software
       package repository <https://packages.nlnetlabs.nl>`_.
       
       To use this repository, add the line below that corresponds to your
       operating system to your :file:`/etc/apt/sources.list` or
       :file:`/etc/apt/sources.list.d/`:

       .. code-block:: text

          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ stretch main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ buster main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ bullseye main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ xenial main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ bionic main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ focal main

       Then run the following commands to add the public key and update the
       repository list:

       .. code-block:: text

          wget -qO- https://packages.nlnetlabs.nl/aptkey.asc | sudo apt-key add -
          sudo apt update

       You can then install RTRTR by running:

       .. code-block:: bash

          sudo apt install rtrtr

       You can now configure RTRTR by editing :file:`/etc/rtrtr.conf` and start
       it with ``sudo systemctl enable --now rtrtr``. 
       
       You can check the status of RTRTR with:
       
       .. code-block:: bash 
       
          sudo systemctl status rtrtr
       
       You can view the logs with: 
       
       .. code-block:: bash
       
          sudo journalctl --unit=rtrtr

   .. group-tab:: RPM Packages

       If you have a machine with an amd64/x86_64 architecture running a
       :abbr:`RHEL (Red Hat Enterprise Linux)`/CentOS 7 or 8 distribution, you
       can install RTRTR from our `software package repository
       <https://packages.nlnetlabs.nl>`_. 
       
       To use this repository, create a file named 
       :file:`/etc/yum.repos.d/nlnetlabs.repo`, enter this configuration and 
       save it:
       
       .. code-block:: text
       
          [nlnetlabs]
          name=NLnet Labs
          baseurl=https://packages.nlnetlabs.nl/linux/centos/$releasever/main/$basearch
          enabled=1
        
       Then run the following command to add the public key:
       
       .. code-block:: bash
       
          sudo rpm --import https://packages.nlnetlabs.nl/aptkey.asc
       
       You can then install RTRTR by running:
        
       .. code-block:: bash
          
          sudo yum install -y rtrtr
           
       You can now configure RTRTR by editing :file:`/etc/rtrtr.conf` and start
       it with ``sudo systemctl enable --now rtrtr``. 
       
       You can check the status of RTRTR with:
       
       .. code-block:: bash 
       
          sudo systemctl status rtrtr
       
       You can view the logs with: 
       
       .. code-block:: bash
       
          sudo journalctl --unit=rtrtr
       
   .. group-tab:: Docker

       To run RTRTR with Docker you will first need to create an
       :file:`rtrtr.conf` file somewhere on your host computer and make that
       available to the Docker container when you run it. For example if your
       config file is in :file:`/etc/rtrtr.conf` on the host computer:

       .. code-block:: bash

          docker run -v /etc/rtrtr.conf:/etc/rtrtr.conf nlnetlabs/rtrtr -c /etc/rtrtr.conf
          
       RTRTR will need network access to fetch and publish data according to the
       configured units and targets respectively. Explaining Docker networking
       is beyond the scope of this quick start, however below are a couple of
       examples to get you started.
       
       If you need an RTRTR unit to fetch data from a source port on the host
       you will also need to give the Docker container access to the host
       network. For example one way to do this is with ``--net=host``, where
       ``...`` represents the rest of the arguments to pass to Docker
       and RTRTR:

       .. code-block:: bash

          docker run --net=host ...
       
       If you're not using ``--net=host`` you will need to tell Docker to 
       expoese the RTRTR target ports, either one by one using ``-p``, or you 
       can publish the default ports exposed by the Docker container (and at the
       same time remap them to high numbered ports) using ``-P``:
       
       .. code-block:: bash

          docker run -p 8080:8080/tcp -p 9001:9001/tcp ...
          
       Or:
       
       .. code-block:: bash

          docker run -P ...
               
   .. group-tab:: Cargo

       Assuming you have a newly installed Debian or Ubuntu machine, you will
       need to install rsync, the C toolchain and Rust. You can then install
       RTRTR:

       .. code-block:: bash

          apt install curl rsync build-essential
          curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
          source ~/.cargo/env
          cargo install --locked rtrtr

       Once RTRTR is installed, you need to create a :ref:`configuration file
       <doc_rtrtr_configuration>` that suits your needs. The config file to use
       needs to be passed to RTRTR via the :option:`-c` option, i.e.:
       
       .. code-block:: text
       
          rtrtr -c rtrtr.conf
       
       If you have an older version of Rust and RTRTR, you can update via:

       .. code-block:: text

          rustup update
          cargo install --locked --force rtrtr

Installing Specific Versions
----------------------------

Before every new release of RTRTR, one or more release candidates are provided
for testing through every installation method. You can also install a specific
version, if needed.

.. tabs::

   .. group-tab:: Deb Packages

       To install release candidates of RTRTR, add the line below that 
       corresponds to your operating system to your ``/etc/apt/sources.list`` or
       ``/etc/apt/sources.list.d/``:

       .. code-block:: text

          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ stretch-proposed main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ buster-proposed main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ bullseye-proposed main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ xenial-proposed main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ bionic-proposed main 
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ focal-proposed main

       You can use this command to get an overview of the available versions:

       .. code-block:: text

          sudo apt policy rtrtr

       You can install a specific version using ``<package name>=<version>``,
       e.g.:

       .. code-block:: text

          sudo apt install rtrtr=0.1.1
          
   .. group-tab:: RPM Packages

       To install release candidates of RTRTR, create an additional repo file
       named :file:`/etc/yum.repos.d/nlnetlabs-testing.repo`, enter this
       configuration and save it:
       
       .. code-block:: text
       
          [nlnetlabs-testing]
          name=NLnet Labs Testing
          baseurl=https://packages.nlnetlabs.nl/linux/centos/$releasever/proposed/$basearch
          enabled=1
        
       You can use this command to get an overview of the available versions:
        
       .. code-block:: bash
        
          sudo yum --showduplicates list rtrtr
          
       You can install a specific version using 
       ``<package name>-<version info>``, e.g.:
         
       .. code-block:: bash
         
          sudo yum install -y rtrtr-0.1.1
             
   .. group-tab:: Docker

       All release versions of RTRTR, as well as release candidates and builds
       based on the latest main branch are available on `Docker Hub
       <https://hub.docker.com/r/nlnetlabs/rtrtr/tags?page=1&ordering=last_updated>`_. 
       
       For example, installing RTRTR 0.1.2 is as simple as:
        
       .. code-block:: text
       
          docker run -it nlnetlabs/rtrtr:v0.1.2
               
   .. group-tab:: Cargo

       All release versions of RTRTR, as well as release candidates, are
       available on `crates.io <https://crates.io/crates/rtrtr/versions>`_, the
       Rust package registry. If you want to install a specific version of RTRTR
       using Cargo, explicitly use the ``--version`` option. If needed, use the
       ``--force`` option to overwrite an existing version:
               
       .. code-block:: text

          cargo install --locked --force rtrtr --version 0.1.2

       All new features of RTRTR are built on a branch and merged via a
       `pull request <https://github.com/NLnetLabs/rtrtr/pulls>`_, allowing
       you to easily try them out using Cargo. If you want to try the a specific
       branch from the repository you can use the ``--git`` and ``--branch``
       options:

       .. code-block:: text

          cargo install --git https://github.com/NLnetLabs/rtrtr.git --branch main
          
       For more installation options refer to the `Cargo book
       <https://doc.rust-lang.org/cargo/commands/cargo-install.html#install-options>`_.

Installing From Source
----------------------

You need a C toolchain and Rust to install and run RTRTR. You can install RTRTR
on any system where you can fulfil these requirements.

C Toolchain
"""""""""""

Some of the libraries RTRTR depends on require a C toolchain to be present.
Your system probably has some easy way to install the minimum set of packages to
build from C sources. For example, this command will install everything you need
on Debian/Ubuntu:

.. code-block:: text

   apt install build-essential

If you are unsure, try to run :command:`cc` on a command line. If there is a
complaint about missing input files, you are probably good to go.

Rust
""""

The Rust compiler runs on, and compiles to, a great number of platforms, though
not all of them are equally supported. The official `Rust Platform Support
<https://doc.rust-lang.org/nightly/rustc/platform-support.html>`_ page provides
an overview of the various support levels.

While some system distributions include Rust as system packages, RTRTR relies on
a relatively new version of Rust, currently 1.47 or newer. We therefore suggest
to use the canonical Rust installation via a tool called :command:`rustup`.

To install :command:`rustup` and Rust, simply do:

.. code-block:: text

   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Alternatively, visit the `official Rust website
<https://www.rust-lang.org/tools/install>`_ for other installation methods.

You can update your Rust installation later by running:

.. code-block:: text

   rustup update

Building
""""""""

The easiest way to get RTRTR is to leave it to Cargo by saying:

.. code-block:: text

   cargo install --locked rtrtr

The command will build RTRTR and install it in the same directory that
Cargo itself lives in, likely ``$HOME/.cargo/bin``. This means RTRTR will
be in your path, too.