# ── Stage 1: Build the Rust gateway binary ────────────────────────────
FROM rust:1.83-bookworm AS builder

WORKDIR /src

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY apps/dashboard/src-tauri apps/dashboard/src-tauri

RUN cargo build --release --bin serialagent

# ── Stage 2: Build the Vue dashboard ─────────────────────────────────
FROM node:22-bookworm-slim AS dashboard

WORKDIR /app

COPY apps/dashboard/package.json apps/dashboard/package-lock.json ./
RUN npm ci

COPY apps/dashboard/ .
RUN npm run build

# ── Stage 3: Minimal runtime image ───────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash serialagent

COPY --from=builder /src/target/release/serialagent /usr/local/bin/serialagent
COPY --from=dashboard /app/dist /srv/dashboard

USER serialagent
WORKDIR /home/serialagent

EXPOSE 3100

ENTRYPOINT ["serialagent"]
