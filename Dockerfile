# -- stage 1: build static rtrtr with musl libc for alpine
FROM alpine:3.13.5 as build

RUN apk add rust cargo

WORKDIR /tmp/rtrtr
COPY . .

RUN cargo build \
    --target x86_64-alpine-linux-musl \
    --release \
    --locked

# -- stage 2: create alpine-based container with the static rtrtr
#             executable
FROM alpine:3.13.5
COPY --from=build /tmp/rtrtr/target/x86_64-alpine-linux-musl/release/rtrtr /usr/local/bin/

# Build variables for uid and guid of user to run container
ARG RUN_USER=rtrtr
ARG RUN_USER_UID=1012
ARG RUN_USER_GID=1012

# Install rsync as rtrtr depends on it
RUN apk add --no-cache rsync libgcc

# Use Tini to ensure that Routinator responds to CTRL-C when run in the
# foreground without the Docker argument "--init" (which is actually another
# way of activating Tini, but cannot be enabled from inside the Docker image).
RUN apk add --no-cache tini
# Tini is now available at /sbin/tini

RUN addgroup -g ${RUN_USER_GID} ${RUN_USER} && \
    adduser -D -u ${RUN_USER_UID} -G ${RUN_USER} ${RUN_USER}

USER $RUN_USER_UID

# Expose the default metrics port
EXPOSE 8080/tcp

# Expose the default data serving port
EXPOSE 9001/tcp

ENTRYPOINT ["/sbin/tini", "--", "rtrtr"]
