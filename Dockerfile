FROM node:20-slim AS frontend
WORKDIR /build
COPY frontend/package.json ./frontend/
RUN npm --prefix frontend install --include=dev
COPY frontend ./frontend
RUN npm --prefix frontend run build

FROM rust:1-slim-bookworm AS rust
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --no-create-home appuser
WORKDIR /app
COPY --from=rust /build/target/release/server /app/server
COPY --from=frontend /build/frontend/dist /app/frontend/dist
ENV STATIC_DIR=/app/frontend/dist
USER appuser
CMD ["/app/server"]