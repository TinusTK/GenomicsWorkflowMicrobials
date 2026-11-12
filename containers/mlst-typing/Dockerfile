# Container of the built-in MLST typing tool (tools/mlst_typing): BLAST from
# bioconda plus the mlst-typing script and its database (PubMLST and
# EnteroBase schemes) at a pinned commit. Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-mlst-typing:<version>.
# Versions match the Galaxy tool tinustk/mlst_typing 0.3.0 (python 3.12, blast 2.16.0).
FROM mambaorg/micromamba:2.0.5
# v0.3.0 (database PubMLST_2026-11 + EnteroBase_2026-11-11)
ARG COMMIT=bbfb926e4a606646793d5a90578f2ce34523d000
RUN micromamba install -y -n base -c conda-forge -c bioconda blast=2.16.0 python=3.12 \
    && micromamba clean -a -y
USER root
ADD https://github.com/TinusTK/mlst-typing/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
# the script and its whole database side by side, as in the Galaxy tool's bundle
RUN mkdir -p /tmp/src /opt/mlst_typing \
    && tar -xzf /tmp/src.tar.gz -C /tmp/src --strip-components=1 \
    && cp /tmp/src/tool/mlst_typing.py /opt/mlst_typing/ \
    && cp -r /tmp/src/database /opt/mlst_typing/database \
    && chmod -R a+rX /opt/mlst_typing \
    && rm -rf /tmp/src /tmp/src.tar.gz
# a run uses the computer's own user, not the image's
ENV PATH=/opt/conda/bin:$PATH
ENTRYPOINT []
