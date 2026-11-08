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
  cache-from = ["type=gha,scope=relayix-build"]
}

target "production" {
  inherits = ["common"]
  target = "production"
  tags = ["relayix:candidate"]
  output = ["type=oci,dest=/tmp/relayix-candidate.tar"]
  attest = ["type=provenance,mode=max", "type=sbom"]
}

target "verified" {
  inherits = ["common"]
  target = "verified"
  tags = ["relayix:verified"]
  output = ["type=docker"]
  cache-to = ["type=gha,scope=relayix-build,mode=max"]
}
