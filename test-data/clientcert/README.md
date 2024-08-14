# CA for testing client certificates

This directory contains keys and certificates for testing client
certificate support.

## Directory Content

The files are as follows. They are all in PEM format.

* **ca.key**: CA private key.
* **ca.cer**: CA certificate for “RTRTR Client Test CA.”
* **subca.key**: Subordinate CA private key.
* **subca.csr**: Subordinate CA CSR.
* **subca.cer**: Subordinate CA certificate
* **client.key**: client private key.
* **client.csr**: client CSR.
* **client.cer**: client certificate for “RTRTR Test Client.”
* **client-combined.pem**: key and certificate for the client and
  subordinate CA certificate.

There are a few additional supporting files:

* **client-extension.txt**: defines the certificate extensions for the
  client certificate.


## Making Certificates

1. Generate a key:

```
openssl ecparam -name prime256v1 -genkey -noout -out $TARGET.key
```

2. Generate a certificate signing request for the key:

```
openssl req -new -sha256 -key $TARGET.key -out $TARGET.csr
```

3a. Generate a CA certificate:

```
openssl x509 -req  -CA ca.cer -CAkey ca.key -CAcreateserial -days 1000000 \
-sha256 -extfile subca-extension.txt -in $TARGET.csr -out $TARGET.cer
```

3b. Generate a client certificate:

```
openssl x509 -req  -CA subca.cer -CAkey subca.key -CAcreateserial \
-days 1000000 -sha256 -extfile client-extension.txt \
-in $TARGET.csr -out $TARGET.cer
```

