#!/usr/bin/env bash

set -eo pipefail
set -x

case $1 in
  post-install)
    echo -e "\nRTRTR VERSION:"
    rtrtr --version

    echo -e "\nRTRTR CONF:"
    cat /etc/rtrtr.conf

    echo -e "\nRTRTR HOME DIR:"
    ls -la /var/lib/rtrtr

    echo -e "\nRTRTR SERVICE STATUS BEFORE ENABLE:"
    systemctl status rtrtr || true

    echo -e "\nENABLE RTRTR SERVICE:"
    systemctl enable rtrtr

    echo -e "\nRTRTR SERVICE STATUS AFTER ENABLE:"
    systemctl status rtrtr || true

    echo -e "\nSTART RTRTR SERVICE:"
    systemctl start rtrtr
        
    sleep 15s
    echo -e "\nRTRTR LOGS AFTER START:"
    journalctl --unit=rtrtr

    echo -e "\nRTRTR SERVICE STATUS AFTER START:"
    systemctl status rtrtr
    ;;

  post-upgrade)
    ;;
esac
