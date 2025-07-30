# Build stage
FROM rust:1.88.0-bookworm AS builder

WORKDIR /usr/src/app
COPY . .

# Build the application in release mode
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install necessary runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from the builder stage
COPY --from=builder /usr/src/app/target/release/co-planet-signaling-server /app/

# Expose port 3000 that the server listens on
EXPOSE 3000

# Run the server
CMD ["./co-planet-signaling-server"]