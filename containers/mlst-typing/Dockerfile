# Container of the built-in MLST typing tool (tools/mlst_typing): BLAST from
# bioconda plus the mlst-typing scripts and its database (PubMLST and
# EnteroBase schemes) at a pinned commit. Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-mlst-typing:<version>.
# Versions match the Galaxy tool tinustk/mlst_typing 0.5.0 (python 3.12, blast 2.16.0).
FROM mambaorg/micromamba:2.0.5
# v0.5.0 (database PubMLST_2026-11 + EnteroBase_2026-11-11)
ARG COMMIT=4031c74515e3559d1842741515fbdd18d5b6650c
RUN micromamba install -y -n base -c conda-forge -c bioconda blast=2.16.0 python=3.12 \
    && micromamba clean -a -y
USER root
ADD https://github.com/TinusTK/mlst-typing/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
# the script and its whole database side by side, as in the Galaxy tool's
# bundle. Unlike that bundle, the two setup scripts come along: the app uses
# them for the user's own PubMLST key and up-to-date database (Databases
# page); the database folder keeps schemes.tsv, which that download reads.
RUN mkdir -p /tmp/src /opt/mlst_typing \
    && tar -xzf /tmp/src.tar.gz -C /tmp/src --strip-components=1 \
    && cp /tmp/src/tool/mlst_typing.py /opt/mlst_typing/ \
    && for f in mlst_typing_setup.py mlst_typing_update.py; do \
         if [ -f /tmp/src/tool/$f ]; then cp /tmp/src/tool/$f /opt/mlst_typing/; fi; done \
    && cp -r /tmp/src/database /opt/mlst_typing/database \
    && chmod -R a+rX /opt/mlst_typing \
    && rm -rf /tmp/src /tmp/src.tar.gz
# a run uses the computer's own user, not the image's
ENV PATH=/opt/conda/bin:$PATH
ENTRYPOINT []
