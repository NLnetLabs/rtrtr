.. _doc_rtrtr_installation:

Installation
============

Getting started with RTRTR is really easy by either installing a Debian and
Ubuntu package, using Docker, or building from :abbr:`Cargo (Rust's build system
and package manager)`.

Quick Start
-----------

.. tabs::

   .. tab:: Packages

       Assuming you have a machine running a recent Debian or Ubuntu distribution, you
       can install RTRTR from our `software package repository
       <https://packages.nlnetlabs.nl>`_. To use this repository, add the line below
       that corresponds to your operating system to your ``/etc/apt/sources.list`` or
       ``/etc/apt/sources.list.d/``:

       .. code-block:: text

          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ stretch main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ buster main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ xenial main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ bionic main
          deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ focal main

       Then run the following commands:

       .. code-block:: text

          sudo apt update && apt-get install -y gnupg2
          wget -qO- https://packages.nlnetlabs.nl/aptkey.asc | sudo apt-key add -
          sudo apt update

       You can then install RTRTR using:

       .. code-block:: bash

          sudo apt install rtrtr

       You can now configure RTRTR by editing :file:`/etc/rtrtr.conf` and start
       it with ``sudo systemctl enable --now rtrtr``. You can check the status
       with the command ``sudo systemctl status rtrtr`` and view the logs with
       ``sudo journalctl --unit=rtrtr``.

   .. tab:: Docker

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
       network. For example one way to do this is with ``--net=host``:

       .. code-block:: bash

          docker run --net=host ...
          
       *(where ``...`` represents the rest of the arguments to pass to Docker
       and RTRTR)*
       
       If you're not using ``--net=host`` you will need to tell Docker to 
       expoese the RTRTR target ports, either one by one using ``-p``, or you 
       can publish the default ports exposed by the Docker container (and at the
       same time remap them to high numbered ports) using ``-P``:
       
       .. code-block:: bash

          docker run -p 8080:8080/tcp -p 9001:9001/tcp ...
          
       Or:
       
       .. code-block:: bash

          docker run -P ...
               
   .. tab:: Cargo

       Assuming you have a newly installed Debian or Ubuntu machine, you will need to
       install rsync, the C toolchain and Rust. You can then install RTRTR:

       .. code-block:: bash

          apt install curl rsync build-essential
          curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
          source ~/.cargo/env
          cargo install --locked rtrtr

       Once RTRTR is installed, you need to create a config file that suits your
       needs. The :file:`etc/rtrtr.conf` example in the `repository
       <https://github.com/NLnetLabs/rtrtr/blob/main/etc/rtrtr.conf>`_ may be a
       good way to start. The config file to use needs to be passed to RTRTR via
       the :option:`-c` option, i.e.:
       
       .. code-block:: text
       
          rtrtr -c rtrtr.conf
       
       If you have an older version of Rust and RTRTR, you can update via:

       .. code-block:: text

          rustup update
          cargo install --locked --force rtrtr

       If you want to try the main branch from the repository instead of a
       release version, you can run:

       .. code-block:: text

          cargo install --git https://github.com/NLnetLabs/rtrtr.git --branch main

System Requirements
-------------------

When choosing a system to run RTRTR on, make sure you have 1GB of available
memory and 1GB of disk space. 

Installing From Source
----------------------

You need a C toolchain and Rust to install and run RTRTR. You can install RTRTR
on any system where you can fulfil these requirements.

C Toolchain
"""""""""""

Some of the libraries Routinator depends on require a C toolchain to be present.
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

While some system distributions include Rust as system packages,
Routinator relies on a relatively new version of Rust, currently 1.45 or
newer. We therefore suggest to use the canonical Rust installation via a
tool called :command:`rustup`.

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

The easiest way to get Routinator is to leave it to Cargo by saying:

.. code-block:: text

   cargo install --locked rtrtr

The command will build Routinator and install it in the same directory that
Cargo itself lives in, likely ``$HOME/.cargo/bin``. This means RTRTR will
be in your path, too.

Installing Specific Versions
----------------------------

Release Candidates of RTRTR are also available on our `software package
repository <https://packages.nlnetlabs.nl>`_. To install these as well, add the
line below that corresponds to your operating system to your
``/etc/apt/sources.list`` or ``/etc/apt/sources.list.d/``:
       
.. code-block:: text

   deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ stretch-proposed main
   deb [arch=amd64] https://packages.nlnetlabs.nl/linux/debian/ buster-proposed main
   deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ xenial-proposed main
   deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ bionic-proposed main
   deb [arch=amd64] https://packages.nlnetlabs.nl/linux/ubuntu/ focal-proposed main

You can use this command to get an overview of the available versions:

.. code-block:: text

   apt policy rtrtr

If you want to install a Release Candidate or a specific version of Routinator
using Cargo, explicitly use the ``--version`` option. If needed, use the
``--force`` option to overwrite an existing version:
        
.. code-block:: text

   cargo install --locked --force rtrtr --version 0.1.2-rc1

If you want to try the main branch from the repository instead of a release
version, you can run:

.. code-block:: text

   cargo install --git https://github.com/NLnetLabs/rtrtr.git --branch main

