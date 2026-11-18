# Container of the built-in Kun-peng tool (tools/kun_peng): the kun_peng
# program built from its source (https://github.com/eric9n/Kun-peng) at a
# pinned commit, plus Bracken (its abundance estimate is a switch of the tool).
# There is no BioContainers image of Kun-peng. Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-kun-peng:<version>.
FROM rust:1-bookworm AS build
# v0.7.13
ARG COMMIT=034dabb547956be472a412a1521a207c30afef99
ADD https://github.com/eric9n/Kun-peng/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
RUN mkdir /src \
    && tar -xzf /tmp/src.tar.gz -C /src --strip-components=1 \
    && cd /src \
    && cargo build --release --locked --bin kun_peng \
    && cp target/release/kun_peng /kun_peng
# A second build with one line changed, for the tool's setting "Hit groups
# counted: Kraken2's way". Kun-peng 0.7.13 checks "minimum hit groups" against
# the number of k-mers in the read (hits.capacity()), so nearly every read
# passes; Kraken2 counts the minimizers that matched the database. With this
# line Kun-peng leaves the same reads unclassified as Kraken2 does. The
# program as released stays the default. The build stops if the line is gone
# (a newer Kun-peng): look again before changing the commit.
RUN cd /src \
    && test "$(grep -c 'let hit_groups = hits.capacity();' src/classify.rs)" = 1 \
    && sed -i 's/let hit_groups = hits.capacity();/let hit_groups = hits.rows.len();/' src/classify.rs \
    && cargo build --release --locked --bin kun_peng \
    && cp target/release/kun_peng /kun_peng_k2

# bash, flock and the usual file commands (the tool's prepare.sh) are in the base image
FROM debian:bookworm-slim
# Bracken 3.0, the version of the app's Bracken tool: its abundance estimate
# is one Python script (standard library only) that reads a Kraken-style report
ARG BRACKEN_COMMIT=38edbc5f7ac4e27dd90c12e9aee6ec01b248731e
ADD https://github.com/jenniferlu717/Bracken/archive/${BRACKEN_COMMIT}.tar.gz /tmp/bracken.tar.gz
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /tmp/bracken /opt/bracken \
    && tar -xzf /tmp/bracken.tar.gz -C /tmp/bracken --strip-components=1 \
    && cp /tmp/bracken/src/est_abundance.py /tmp/bracken/LICENSE /opt/bracken/ \
    && chmod -R a+rX /opt/bracken \
    && rm -rf /tmp/bracken /tmp/bracken.tar.gz
COPY --from=build /kun_peng /kun_peng_k2 /usr/local/bin/
ENTRYPOINT []
