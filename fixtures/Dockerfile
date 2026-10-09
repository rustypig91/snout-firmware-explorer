# Build from this directory:
# docker build --platform linux/amd64 -t snout-fixtures .
FROM ubuntu:26.04

# Classic TI Arm CGT is distributed for x86_64 Linux hosts. On an Arm host,
# select linux/amd64 explicitly and use Docker's platform emulation.
RUN test "$(dpkg --print-architecture)" = amd64 || \
    (echo 'This image requires --platform linux/amd64 for classic TI Arm CGT.' >&2; exit 1)
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl xz-utils python3 cmake ninja-build \
    clang-21 lld-21 libstdc++6 libtinfo6 \
    && rm -rf /var/lib/apt/lists/*

# Arm GNU Toolchain 14.2.rel1; preserve the original GCC reference version.
# https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads
RUN curl -fL --retry 3 \
    https://developer.arm.com/-/media/Files/downloads/gnu/14.2.rel1/binrel/arm-gnu-toolchain-14.2.rel1-x86_64-arm-none-eabi.tar.xz \
    -o /tmp/gcc.tar.xz \
    && echo '62a63b981fe391a9cbad7ef51b17e49aeaa3e7b0d029b36ca1e9c3b2a9b78823  /tmp/gcc.tar.xz' | sha256sum -c - \
    && mkdir -p /opt/gcc \
    && tar -xJf /tmp/gcc.tar.xz -C /opt/gcc --strip-components=1 \
    && rm /tmp/gcc.tar.xz

# TI's official installers contain their license/manifest files. Checksums come
# from the linked download pages; compiler installations stay inside the image.
# https://www.ti.com/tool/download/ARM-CGT/20.2.7.LTS
RUN curl -fL --retry 3 \
    https://dr-download.ti.com/software-development/ide-configuration-compiler-or-debugger/MD-sDOoXkUcde/20.2.7.LTS/ti_cgt_tms470_20.2.7.LTS_linux-x64_installer.bin \
    -o /tmp/ti-arm.bin \
    && echo '697bc381ed00c52d7be19d67f1cdf651a3068ec7c38404edd696ee7f801fb3e8  /tmp/ti-arm.bin' | sha256sum -c - \
    && chmod +x /tmp/ti-arm.bin \
    && /tmp/ti-arm.bin --mode unattended --prefix /opt/ti \
    && rm /tmp/ti-arm.bin

# https://www.ti.com/tool/download/ARM-CGT-CLANG/4.0.3.LTS
RUN curl -fL --retry 3 \
    https://dr-download.ti.com/software-development/ide-configuration-compiler-or-debugger/MD-ayxs93eZNN/4.0.3.LTS/ti_cgt_armllvm_4.0.3.LTS_linux-x64_installer.bin \
    -o /tmp/ti-clang.bin \
    && echo '617b0379a0f0aaac364d6b16f0ea899687ebf7f773a840d1c7d6f0691205919c  /tmp/ti-clang.bin' | sha256sum -c - \
    && chmod +x /tmp/ti-clang.bin \
    && /tmp/ti-clang.bin --mode unattended --prefix /opt/ti \
    && rm /tmp/ti-clang.bin

ENV PATH="/opt/gcc/bin:/opt/ti/ti-cgt-arm_20.2.7.LTS/bin:/opt/ti/ti-cgt-armllvm_4.0.3.LTS/bin:/usr/lib/llvm-21/bin:${PATH}"
RUN arm-none-eabi-gcc --version && clang --version && ld.lld --version \
    && armcl --help && tiarmclang --version

WORKDIR /workspace
COPY . /workspace/fixtures
# Fail image construction if any installed compiler cannot build the fixtures.
RUN python3 fixtures/generate.py --toolchain all --output-dir /tmp/fixture-smoke \
    && rm -rf /tmp/fixture-smoke
ENTRYPOINT ["python3", "/workspace/fixtures/generate.py"]
CMD ["--toolchain", "all", "--output-dir", "/output"]
