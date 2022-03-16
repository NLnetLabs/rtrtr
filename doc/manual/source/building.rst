Building From Source
====================

In addition to meeting the :ref:`system requirements <installation:System
Requirements>`, there are two things you need to build RTRTR: a
C toolchain and Rust. You can run RTRTR on any operating system and CPU
architecture where you can fulfil these requirements.

Dependencies
------------

C Toolchain
"""""""""""

Some of the libraries RTRTR depends on require a C toolchain to be present.
Your system probably has some easy way to install the minimum set of packages
to build from C sources. For example, this command will install everything
you need on Debian/Ubuntu:

.. code-block:: text

  apt install build-essential

If you are unsure, try to run :command:`cc` on a command line. If there is a
complaint about missing input files, you are probably good to go.

Rust
""""

The Rust compiler runs on, and compiles to, a great number of platforms,
though not all of them are equally supported. The official `Rust Platform
Support`_ page provides an overview of the various support levels.

While some system distributions include Rust as system packages, RTRTR
relies on a relatively new version of Rust, currently 1.52 or newer. We
therefore suggest to use the canonical Rust installation via a tool called
:program:`rustup`.

Assuming you already have :program:`curl` installed, you can install
:program:`rustup` and Rust by simply entering:

.. code-block:: text

  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Alternatively, visit the `Rust website
<https://www.rust-lang.org/tools/install>`_ for other installation methods.

Building and Updating
---------------------

In Rust, a library or executable program such as RTRTR is called a
*crate*. Crates are published on `crates.io
<https://crates.io/crates/rtrtr>`_, the Rust package registry. Cargo is
the Rust package manager. It is a tool that allows Rust packages to declare
their various dependencies and ensure that you’ll always get a repeatable
build. 

Cargo fetches and builds RTRTR’s dependencies into an executable binary
for your platform. By default you install from crates.io, but you can for
example also install from a specific Git URL, as explained below.

Installing the latest RTRTR release from crates.io is as simple as
running:

.. code-block:: text

  cargo install --locked rtrtr

The command will build RTRTR and install it in the same directory that
Cargo itself lives in, likely ``$HOME/.cargo/bin``. This means RTRTR
will be in your path, too.

Updating
""""""""

If you want to update to the latest version of RTRTR, it’s recommended
to update Rust itself as well, using:

.. code-block:: text

    rustup update

Use the ``--force`` option to overwrite an existing version with the latest
RTRTR release:

.. code-block:: text

    cargo install --locked --force rtrtr

Once RTRTR is installed, you need to create a :doc:`configuration` file that
suits your needs. The config file to use needs to be passed to RTRTR via the
:option:`-c` option, i.e.:
       
.. code-block:: text

    rtrtr -c rtrtr.conf

Installing Specific Versions
""""""""""""""""""""""""""""

If you want to install a specific version of
RTRTR using Cargo, explicitly use the ``--version`` option. If needed,
use the ``--force`` option to overwrite an existing version:
        
.. code-block:: text

    cargo install --locked --force rtrtr --version 0.2.0-rc2

All new features of RTRTR are built on a branch and merged via a `pull
request <https://github.com/NLnetLabs/rtrtr/pulls>`_, allowing you to
easily try them out using Cargo. If you want to try a specific branch from
the repository you can use the ``--git`` and ``--branch`` options:

.. code-block:: text

    cargo install --git https://github.com/NLnetLabs/rtrtr.git --branch main
    
.. Seealso:: For more installation options refer to the `Cargo book
             <https://doc.rust-lang.org/cargo/commands/cargo-install.html#install-options>`_.

Platform Specific Instructions
------------------------------

For some platforms, :program:`rustup` cannot provide binary releases to
install directly. The `Rust Platform Support`_ page lists
several platforms where official binary releases are not available, but Rust
is still guaranteed to build. For these platforms, automated tests are not
run so it’s not guaranteed to produce a working build, but they often work to
quite a good degree.

.. _Rust Platform Support:  https://doc.rust-lang.org/nightly/rustc/platform-support.html

OpenBSD
"""""""

On OpenBSD, `patches
<https://github.com/openbsd/ports/tree/master/lang/rust/patches>`_ are
required to get Rust running correctly, but these are well maintained and
offer the latest version of Rust quite quickly.

Rust can be installed on OpenBSD by running:

.. code-block:: bash

   pkg_add rust

CentOS 6
""""""""

The standard installation method does not work when using CentOS 6. Here, you
will end up with a long list of error messages about missing assembler
instructions. This is because the assembler shipped with CentOS 6 is too old.

You can get the necessary version by installing the `Developer Toolset 6
<https://www.softwarecollections.org/en/scls/rhscl/devtoolset-6/>`_ from the
`Software Collections
<https://wiki.centos.org/AdditionalResources/Repositories/SCL>`_ repository.
On a virgin system, you can install Rust using these steps:

.. code-block:: bash

   sudo yum install centos-release-scl
   sudo yum install devtoolset-6
   scl enable devtoolset-6 bash
   curl https://sh.rustup.rs -sSf | sh
   source $HOME/.cargo/env
