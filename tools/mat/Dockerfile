# Middleware Atlas Eclipse MAT worker image.
# MAT 1.17.0 (20260601 build) requires Java 21 or later.
# Official multi-architecture manifest-list digest for eclipse-temurin:25-jre-jammy.
# Update only after reviewing the official image tag and all supported architectures.
ARG TEMURIN_BASE=docker.io/library/eclipse-temurin:25-jre-jammy@sha256:25777acfabf927084b7ef46d8bc786b6203c8c344f56541054238b8c4fe73db9
FROM ${TEMURIN_BASE}
ARG TARGETARCH
ARG MAT_VERSION=1.17.0
ARG MAT_BUILD=20260601
LABEL org.opencontainers.image.title="Middleware Atlas Eclipse MAT worker"
LABEL org.opencontainers.image.description="Headless batch Eclipse Memory Analyzer worker"
LABEL org.opencontainers.image.source="https://github.com/ramu11/middleware-atlas"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip libgtk-3-0 libx11-6 libxtst6 libxrender1 libxext6 libxi6 libglib2.0-0 libnss3 \
    && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "${TARGETARCH:-amd64}" in \
      amd64) MAT_PLATFORM=x86_64 ;; \
      arm64) MAT_PLATFORM=aarch64 ;; \
      ppc64le) MAT_PLATFORM=ppc64le ;; \
      *) echo "Unsupported MAT container architecture: ${TARGETARCH}" >&2; exit 2 ;; \
    esac; \
    FILE="MemoryAnalyzer-${MAT_VERSION}.${MAT_BUILD}-linux.gtk.${MAT_PLATFORM}.zip"; \
    URL="https://www.eclipse.org/downloads/download.php?file=/mat/${MAT_VERSION}/rcp/${FILE}&r=1"; \
    SUMS_URL="https://www.eclipse.org/downloads/sums.php?file=%2Fmat%2F${MAT_VERSION}%2Frcp%2F${FILE}&type=sha512"; \
    curl --fail --location --retry 3 --retry-delay 2 "$URL" -o /tmp/mat.zip; \
    curl --fail --location --retry 3 --retry-delay 2 "$SUMS_URL" -o /tmp/mat.sums; \
    EXPECTED_SHA512="$(grep -Eo '[A-Fa-f0-9]{128}' /tmp/mat.sums | head -n 1)"; \
    test "${#EXPECTED_SHA512}" -eq 128 || { echo "Could not obtain Eclipse's published SHA-512 for ${FILE}" >&2; exit 12; }; \
    printf '%s  %s\n' "$EXPECTED_SHA512" /tmp/mat.zip | sha512sum --check --status || { echo "Eclipse MAT ZIP checksum verification failed" >&2; exit 13; }; \
    mkdir -p /opt/mat; \
    unzip -q /tmp/mat.zip -d /opt/mat; \
    find /opt/mat -type f -name 'org.eclipse.equinox.launcher_*.jar' -print -quit | grep -q .; \
    rm -f /tmp/mat.zip /tmp/mat.sums
COPY mat-job /usr/local/bin/mat-job
RUN chmod 0555 /usr/local/bin/mat-job && find /opt/mat -type d -exec chmod 0555 {} +
ENV MAT_JAVA_MAX_HEAP=4g
WORKDIR /work
ENTRYPOINT ["/usr/local/bin/mat-job"]
