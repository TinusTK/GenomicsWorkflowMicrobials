# Container of the built-in S. pyogenes typing tool (tools/spyogenes_typing):
# BLAST from bioconda plus the spyogenes-typing scripts and its PubMLST
# database at a pinned commit. Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-spyogenes-typing:<version>.
# Versions match the Galaxy tool tinustk/spyogenes_typing 1.0.0+galaxy0 (python 3.12, blast 2.16.0).
FROM mambaorg/micromamba:2.0.5
# v1.0.0+galaxy0 (database PubMLST_2026-11-20)
ARG COMMIT=776a1fcd56d42c39326c18dfed3f30830a32c723
RUN micromamba install -y -n base -c conda-forge -c bioconda blast=2.16.0 python=3.12 \
    && micromamba clean -a -y
USER root
ADD https://github.com/TinusTK/spyogenes-typing/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
# the script and its whole database side by side, as in the Galaxy tool's
# bundle. Unlike that bundle, the two setup scripts come along: the app uses
# them for the user's own PubMLST key and up-to-date database (Databases
# page); the database folder keeps schemes.tsv, which that download reads.
RUN mkdir -p /tmp/src /opt/spyogenes_typing \
    && tar -xzf /tmp/src.tar.gz -C /tmp/src --strip-components=1 \
    && cp /tmp/src/tool/spyogenes_typing.py /tmp/src/tool/spyogenes_typing_setup.py \
          /tmp/src/tool/spyogenes_typing_update.py /opt/spyogenes_typing/ \
    && cp -r /tmp/src/database /opt/spyogenes_typing/database \
    && chmod -R a+rX /opt/spyogenes_typing \
    && rm -rf /tmp/src /tmp/src.tar.gz
# a run uses the computer's own user, not the image's
ENV PATH=/opt/conda/bin:$PATH
ENTRYPOINT []
