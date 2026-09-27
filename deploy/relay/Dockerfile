# Voltip relay: forwards ciphertext between paired devices; never sees plaintext.
FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p voltip-relay --locked

FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/voltip-relay /usr/local/bin/voltip-relay
ENV VOLTIP_RELAY_BIND=0.0.0.0:47830 VOLTIP_RELAY_JSON_LOGS=1 RUST_LOG=info
EXPOSE 47830
USER nonroot
ENTRYPOINT ["/usr/local/bin/voltip-relay"]
