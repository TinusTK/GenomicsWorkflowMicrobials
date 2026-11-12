# Container of the built-in C. diphtheriae typing tool (tools/cdiphtheriae_typing):
# BLAST from bioconda plus the typing script and its BIGSdb-Pasteur database at
# a pinned commit. Built and published by .github/workflows/containers.yml as
# ghcr.io/tinustk/gwm-cdiphtheriae-typing:<version>.
# Versions match the Galaxy tool tinustk/cdiphtheriae_typing 1.1.0+galaxy0.
FROM mambaorg/micromamba:2.0.5
# v1.1.0+galaxy0 (database Pasteur_2026-10b)
ARG COMMIT=0a8c0e35fdbe564a1e2557b14b8b76f99d19feb3
RUN micromamba install -y -n base -c conda-forge -c bioconda blast=2.16.0 python=3.12 \
    && micromamba clean -a -y
USER root
ADD https://github.com/TinusTK/corynebacterium-diphtheriae-typing/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
RUN mkdir -p /tmp/src /opt/cdiphtheriae_typing \
    && tar -xzf /tmp/src.tar.gz -C /tmp/src --strip-components=1 \
    && cp -r /tmp/src/tool/diphtheria_typing.py /tmp/src/database /opt/cdiphtheriae_typing/ \
    && chmod -R a+rX /opt/cdiphtheriae_typing \
    && rm -rf /tmp/src /tmp/src.tar.gz
# a run uses the computer's own user, not the image's
ENV PATH=/opt/conda/bin:$PATH
ENTRYPOINT []
