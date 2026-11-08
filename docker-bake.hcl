// One BuildKit solve shares the exact runtime graph between both exports.
variable "IMAGE" { default = "ghcr.io/opsforge89/relayix" }
variable "REVISION" { default = "local" }
variable "VERSION" { default = "dev" }
variable "REPOSITORY" { default = "opsforge89/Relayix" }

group "default" {
  targets = ["production", "verified"]
}

target "common" {
  context = "."
  dockerfile = "Dockerfile"
  platforms = ["linux/amd64"]
  pull = true
  labels = {
    "org.opencontainers.image.source" = "https://github.com/${REPOSITORY}"
    "org.opencontainers.image.revision" = REVISION
    "org.opencontainers.image.version" = VERSION
  }
  cache-from = ["type=registry,ref=${IMAGE}:buildcache"]
}

target "production" {
  inherits = ["common"]
  target = "production"
  tags = ["${IMAGE}:sha-${REVISION}"]
  output = ["type=registry"]
  attest = ["type=provenance,mode=max", "type=sbom"]
}

target "verified" {
  inherits = ["common"]
  target = "verified"
  tags = ["relayix:verified"]
  output = ["type=docker"]
  cache-to = ["type=registry,ref=${IMAGE}:buildcache,mode=max,image-manifest=true,oci-mediatypes=true"]
}
