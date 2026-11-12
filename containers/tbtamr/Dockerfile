# Container of the built-in tbtAMR tool (tools/tbtamr): tbtAMR and mutAMR from
# PyPI plus the programs mutAMR runs, at the exact builds of tbtAMR's own
# environment.yml (https://github.com/MDU-PHL/tbtamr). Built and published by
# .github/workflows/containers.yml as ghcr.io/tinustk/gwm-tbtamr:<version>.
#
# Why not the BioContainers image: in quay.io/biocontainers/tbtamr 1.0.3 and
# 1.0.4 freebayes 1.3.8 crashes (signal 11 in libvcflib) when it writes its
# first variant. mutAMR does not notice, no SNP is called, and every drug is
# reported "Susceptible". The builds pinned here are the ones the tool was
# validated with. After any change, check that a run on real reads has SNPs
# in <sample>/<sample>.annot.vcf.gz and that a known resistant sample is
# reported resistant.
FROM mambaorg/micromamba:2.0.5
RUN micromamba install -y -n base -c conda-forge -c bioconda \
        python=3.10.14 pip \
        bwa=0.7.18=he4a0461_1 \
        samtools=1.20=h50ea8bc_1 \
        bcftools=1.20=h8b25389_1 \
        htslib=1.21=h5efdd21_0 \
        freebayes=1.3.8=h6a68c12_2 \
        vcflib=1.0.10=hdcf5f25_1 \
        tabixpp=1.1.2=h27d5293_3 \
        wfa2-lib=2.3.5=h4ac6f70_2 \
        delly=1.2.8=hf9970c3_0 \
        samclip=0.4.0=hdfd78af_1 \
        snpeff=5.2=hdfd78af_1 \
        openjdk=22.0.1 \
        parallel=20240722 \
        csvtk=0.30.0 \
    && micromamba clean -a -y
ARG MAMBA_DOCKERFILE_ACTIVATE=1
RUN pip install --no-cache-dir \
        tbtamr==1.0.4 mutamr==0.0.2 \
        pandas==2.2.2 numpy==2.1.1 pysam==0.22.1 pydantic==2.9.2 \
        tabulate==0.9.0 unidecode==1.3.8 joblib==1.4.2 tqdm==4.66.5 requests==2.32.3
# a run uses the computer's own user, not the image's
USER root
RUN chmod -R a+rX /opt/conda
ENV PATH=/opt/conda/bin:$PATH
ENTRYPOINT []
