# Tells docker to use the latest Rust official image
FROM rust:1-bookworm AS builder

WORKDIR /usr/src/app
COPY . .
# Will build and cache the binary and dependent crates in release mode
RUN --mount=type=cache,target=/usr/local/cargo,from=rust:latest,source=/usr/local/cargo \
    --mount=type=cache,target=target \
    cargo build --release && mv ./target/release/typst-flower ./typst-flower

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl-dev && rm -rf /var/lib/apt/lists/*
# Run as "app" user
RUN useradd -ms /bin/bash app

USER app
WORKDIR /app

# Get compiled binaries from builder's cargo install directory
COPY --from=builder /usr/src/app/typst-flower /app/typst-flower

# Create a .env file for dotenv
RUN touch .env

# Expose the port our app is running on
EXPOSE 3030

# Run the app
CMD ./typst-flower