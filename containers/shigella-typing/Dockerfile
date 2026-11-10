# Container of the built-in Shigella typing tool (tools/shigella_typing):
# ShigaPass, Mykrobe and BLAST from bioconda plus the shigella-typing code and
# its Mykrobe panels at a pinned commit. Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-shigella-typing:<version>.
# Versions match the Galaxy tool tinustk/shigella_typing 1.0.2.
FROM mambaorg/micromamba:2.0.5
# v1.0.2
ARG COMMIT=05cd267bea567f322c4d13c67ed76c34f75bbf02
RUN micromamba install -y -n base -c conda-forge -c bioconda \
        python=3.12 mykrobe=0.13.0 setuptools=80.10.2 shigapass=1.5.0 blast=2.17.0 \
        bash coreutils sed gawk grep \
    && micromamba clean -a -y
USER root
ADD https://github.com/TinusTK/shigella-typing/archive/${COMMIT}.tar.gz /tmp/src.tar.gz
RUN mkdir -p /tmp/src /opt/shigella_typing \
    && tar -xzf /tmp/src.tar.gz -C /tmp/src --strip-components=1 \
    && cp -r /tmp/src/tool/shigella_typing /tmp/src/panels /opt/shigella_typing/ \
    && chmod -R a+rX /opt/shigella_typing \
    && rm -rf /tmp/src /tmp/src.tar.gz
# a run uses the computer's own user, not the image's
ENV PATH=/opt/conda/bin:$PATH PYTHONPATH=/opt/shigella_typing SHIGELLA_PANELS=/opt/shigella_typing/panels
ENTRYPOINT []
