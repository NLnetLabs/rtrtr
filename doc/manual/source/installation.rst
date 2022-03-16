Installation
============

System Requirements
-------------------

When choosing a system to run RTRTR on, make sure you have 1GB of available
memory and 1GB of disk space. 

Binary Packages
---------------

Getting started with RTRTR is really easy by installing a binary package
for either Debian and Ubuntu or for Red Hat Enterprise Linux (RHEL) and
compatible systems such as Rocky Linux. Alternatively, you can run with
Docker. 

You can also build RTRTR from the source code using Cargo, Rust's build
system and package manager. Cargo lets you to run RTRTR on almost any
operating system and CPU architecture. Refer to the :doc:`building` section
to get started.

.. tabs::

   .. group-tab:: Debian

       To install an RTRTR package, you need the 64-bit version of one of
       these Debian versions:

         -  Debian Bullseye 11
         -  Debian Buster 10
         -  Debian Stretch 9

       Packages for the ``amd64``/``x86_64`` architecture are available for
       all listed versions. In addition, we offer ``armhf`` architecture
       packages for Debian/Raspbian Bullseye, and ``arm64`` for Buster.
       
       First update the ``apt`` package index: 

       .. code-block:: bash

          sudo apt update

       Then install packages to allow ``apt`` to use a repository over HTTPS:

       .. code-block:: bash

          sudo apt install \
            ca-certificates \
            curl \
            gnupg \
            lsb-release

       Add the GPG key from NLnet Labs:

       .. code-block:: bash

          curl -fsSL https://packages.nlnetlabs.nl/aptkey.asc | sudo gpg --dearmor -o /usr/share/keyrings/nlnetlabs-archive-keyring.gpg

       Now, use the following command to set up the *main* repository:

       .. code-block:: bash

          echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/nlnetlabs-archive-keyring.gpg] https://packages.nlnetlabs.nl/linux/debian \
          $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/nlnetlabs.list > /dev/null

       Update the ``apt`` package index once more: 

       .. code-block:: bash

          sudo apt update

       You can now install RTRTR with:

       .. code-block:: bash

          sudo apt install rtrtr

       :doc:`Configure<configuration>` RTRTR by editing :file:`/etc/rtrtr.conf`
       and start it with:
       
       .. code-block:: bash 
       
          sudo systemctl enable --now rtrtr 
       
       You can check the status of RTRTR with:
       
       .. code-block:: bash 
       
          sudo systemctl status rtrtr
       
       You can view the logs with: 
       
       .. code-block:: bash
       
          sudo journalctl --unit=rtrtr

   .. group-tab:: Ubuntu

       To install an RTRTR package, you need the 64-bit version of one of
       these Ubuntu versions:

         - Ubuntu Focal 20.04 (LTS)
         - Ubuntu Bionic 18.04 (LTS)
         - Ubuntu Xenial 16.04 (LTS)

       Packages are available for the ``amd64``/``x86_64`` architecture only.
       
       First update the ``apt`` package index: 

       .. code-block:: bash

          sudo apt update

       Then install packages to allow ``apt`` to use a repository over HTTPS:

       .. code-block:: bash

          sudo apt install \
            ca-certificates \
            curl \
            gnupg \
            lsb-release

       Add the GPG key from NLnet Labs:

       .. code-block:: bash

          curl -fsSL https://packages.nlnetlabs.nl/aptkey.asc | sudo gpg --dearmor -o /usr/share/keyrings/nlnetlabs-archive-keyring.gpg

       Now, use the following command to set up the *main* repository:

       .. code-block:: bash

          echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/nlnetlabs-archive-keyring.gpg] https://packages.nlnetlabs.nl/linux/ubuntu \
          $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/nlnetlabs.list > /dev/null

       Update the ``apt`` package index once more: 

       .. code-block:: bash

          sudo apt update

       You can now install RTRTR with:

       .. code-block:: bash

          sudo apt install rtrtr

       :doc:`Configure<configuration>` RTRTR by editing :file:`/etc/rtrtr.conf`
       and start it with:

       .. code-block:: bash 
       
          sudo systemctl enable --now rtrtr 
       
       You can check the status of RTRTR with:
       
       .. code-block:: bash 
       
          sudo systemctl status rtrtr
       
       You can view the logs with: 
       
       .. code-block:: bash
       
          sudo journalctl --unit=rtrtr

   .. group-tab:: RHEL/CentOS

       To install an RTRTR package, you need Red Hat Enterprise Linux
       (RHEL) 7 or 8, or compatible operating system such as Rocky Linux.
       Packages are available for the ``amd64``/``x86_64`` architecture only.
       
       First create a file named :file:`/etc/yum.repos.d/nlnetlabs.repo`, enter
       this configuration and save it:
       
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
           
       :doc:`Configure<configuration>` RTRTR by editing :file:`/etc/rtrtr.conf`
       and start it with:

       .. code-block:: bash 
       
          sudo systemctl enable --now rtrtr 
       
       You can check the status of RTRTR with:
       
       .. code-block:: bash 
       
          sudo systemctl status rtrtr
       
       You can view the logs with: 
       
       .. code-block:: bash
       
          sudo journalctl --unit=rtrtr
       
   .. group-tab:: Docker

       RTRTR Docker images are built with Alpine Linux for
       ``amd64``/``x86_64`` architecture.

       To run RTRTR with Docker you will first need to create an
       :file:`rtrtr.conf` file somewhere on your host computer and make that
       available to the Docker container when you run it. For example if your
       config file is in :file:`/etc/rtrtr.conf` on the host computer:

       .. code-block:: bash

          docker run -v /etc/rtrtr.conf:/etc/rtrtr.conf nlnetlabs/rtrtr -c /etc/rtrtr.conf
          
       RTRTR will need network access to fetch and publish data according to the
       configured units and targets respectively. Explaining Docker networking
       is beyond the scope of this Quick Start, however below are a couple of
       examples to get you started.
       
       If you need an RTRTR unit to fetch data from a source port on the host
       you will also need to give the Docker container access to the host
       network. For example one way to do this is with ``--net=host``, where
       ``...`` represents the rest of the arguments to pass to Docker
       and RTRTR:

       .. code-block:: bash

          docker run --net=host ...
       
       If you're not using ``--net=host`` you will need to tell Docker to 
       expose the RTRTR target ports, either one by one using ``-p``, or you 
       can publish the default ports exposed by the Docker container (and at the
       same time remap them to high numbered ports) using ``-P``:
       
       .. code-block:: bash

          docker run -p 8080:8080/tcp -p 9001:9001/tcp ...
          
       Or:
       
       .. code-block:: bash

          docker run -P ...

Updating
--------

.. tabs::

   .. group-tab:: Debian

       To update an existing RTRTR installation, first update the 
       repository using:

       .. code-block:: text

          sudo apt update

       You can use this command to get an overview of the available versions:

       .. code-block:: text

          sudo apt policy rtrtr

       You can upgrade an existing RTRTR installation to the latest version
       using:

       .. code-block:: text

          sudo apt --only-upgrade install rtrtr

   .. group-tab:: Ubuntu

       To update an existing RTRTR installation, first update the 
       repository using:

       .. code-block:: text

          sudo apt update

       You can use this command to get an overview of the available versions:

       .. code-block:: text

          sudo apt policy rtrtr

       You can upgrade an existing RTRTR installation to the latest version
       using:

       .. code-block:: text

          sudo apt --only-upgrade install rtrtr

   .. group-tab:: RHEL/CentOS

       To update an existing RTRTR installation, you can use this command 
       to get an overview of the available versions:
        
       .. code-block:: bash
        
          sudo yum --showduplicates list rtrtr
          
       You can update to the latest version using:
         
       .. code-block:: bash
         
          sudo yum update -y rtrtr
             
   .. group-tab:: Docker

       Upgrading to the latest version of RTRTR can be done with:
        
       .. code-block:: text
       
          docker run -it nlnetlabs/rtrtr:latest

Installing Specific Versions
----------------------------

Before every new release of RTRTR, one or more release candidates are 
provided for testing through every installation method. You can also install
a specific version, if needed.

.. tabs::

   .. group-tab:: Debian

       If you would like to try out release candidates of RTRTR you can add
       the *proposed* repository to the existing *main* repository described
       earlier. 
       
       Assuming you already have followed the steps to install regular releases,
       run this command to add the additional repository:

       .. code-block:: bash

          echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/nlnetlabs-archive-keyring.gpg] https://packages.nlnetlabs.nl/linux/debian \
          $(lsb_release -cs)-proposed main" | sudo tee /etc/apt/sources.list.d/nlnetlabs-proposed.list > /dev/null

       Make sure to update the ``apt`` package index:

       .. code-block:: bash

          sudo apt update
       
       You can now use this command to get an overview of the available 
       versions:

       .. code-block:: bash

          sudo apt policy rtrtr

       You can install a specific version using ``<package name>=<version>``,
       e.g.:

       .. code-block:: bash

          sudo apt install rtrtr=0.1.1~rc2-1buster

   .. group-tab:: Ubuntu

       If you would like to try out release candidates of RTRTR you can add
       the *proposed* repository to the existing *main* repository described
       earlier. 
       
       Assuming you already have followed the steps to install regular releases,
       run this command to add the additional repository:

       .. code-block:: bash

          echo \
          "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/nlnetlabs-archive-keyring.gpg] https://packages.nlnetlabs.nl/linux/ubuntu \
          $(lsb_release -cs)-proposed main" | sudo tee /etc/apt/sources.list.d/nlnetlabs-proposed.list > /dev/null

       Make sure to update the ``apt`` package index:

       .. code-block:: bash

          sudo apt update
       
       You can now use this command to get an overview of the available 
       versions:

       .. code-block:: bash

          sudo apt policy rtrtr

       You can install a specific version using ``<package name>=<version>``,
       e.g.:

       .. code-block:: bash

          sudo apt install rtrtr=0.1.1~rc2-1bionic
          
   .. group-tab:: RHEL/CentOS

       To install release candidates of RTRTR, create an additional repo 
       file named :file:`/etc/yum.repos.d/nlnetlabs-testing.repo`, enter this
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

       All release versions of RTRTR, as well as release candidates and
       builds based on the latest main branch are available on `Docker Hub
       <https://hub.docker.com/r/nlnetlabs/rtrtr/tags?page=1&ordering=last_updated>`_. 
       
       For example, installing RTRTR 0.1.1 is as simple as:
        
       .. code-block:: text
       
          docker run -it nlnetlabs/rtrtr:v0.1.1
