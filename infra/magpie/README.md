# MAGPIE analysis preview — magpie.birdtest.org

MAGPIE owns the app, its WASM build and versioned GitHub releases. Birdtest
owns only this independent static hosting stack and the release uploader.
The release workflow is part of [MAGPIE's preview PR #748](https://github.com/jvc56/MAGPIE/pull/748).
There is no app source or compiled engine checked into Birdtest.

CloudFront serves a private S3 origin with COOP `same-origin`, COEP
`require-corp` and CORP `same-origin` on every asset. The browser can start
WASM threads immediately; it does not have to register a service worker and
reload. Computation and game records stay in the browser. This stack has no
access to Birdtest's accounts, database, contributors or ECS service.

The normal URL redirects to `/releases/<version>/wasmentry/`. Relative module,
worker and data URLs then remain inside that release, including downloads
started by a tab left open during an update. Released files are immutable;
publishing a new default does not remove older versions. Only the root entry
redirect changes. Missing files remain errors, never an HTML fallback.

## 1. Produce a version in MAGPIE

Once the release workflow is on MAGPIE's default branch, run **Release WASM
preview** on the app revision to publish, with a fresh tag such as
`wasm-preview-v0.1.0`. The workflow compiles with a pinned Emscripten version,
downloads only the required data at a pinned MAGPIE-DATA commit, and checks
the packaged app in Chromium with service workers blocked. It exercises
Kibitz, Sim, PEG and Endgame before publishing a GitHub prerelease containing:

- `magpie-wasm-preview.tar.gz`: UI, engine, CSW24/NWL23 KWG/KLV2 and small supporting data.
- `magpie-wasm-preview.tar.gz.sha256`: archive checksum.
- Inside the archive, `release.json`: version, source/data revisions,
  toolchain version and a checksum for every deployable file.

No WMP, RIT or test data is included. Existing release versions must never
be rebuilt or overwritten; choose a new version for each update. The release
is durable, unlike the expiring CI artifact. No AWS credentials are needed in
MAGPIE. Publishing a GitHub release does not deploy it to Birdtest.

## 2. Bootstrap hosting once

Use the owner's normal AWS login and a recent AWS CLI v2. Request an ACM
certificate for **magpie.birdtest.org in us-east-1**, add its DNS validation
CNAME at the existing DNS provider and wait for status `ISSUED`. The existing
Birdtest certificate is usable only if it covers this subdomain and is in
us-east-1.

This directory is a separate Terraform root. Do **not** use the main
`birdtest/terraform.tfstate` key or run `scripts/deploy.sh` for this app.
Create ignored `infra/magpie/backend.tf`, using the same protected state
bucket and region as the main stack, with a **different key**:

```hcl
terraform {
  backend "s3" {
    bucket       = "<existing-state-bucket>"
    key          = "birdtest/magpie/terraform.tfstate"
    region       = "<state-bucket-region>"
    use_lockfile = true
  }
}
```

The S3 lock-file backend requires Terraform 1.10 or newer. Use the same
Terraform version as the other deployers. Create ignored
`infra/magpie/prod.tfvars`:

```hcl
domain_name     = "magpie.birdtest.org"
certificate_arn = "<issued-us-east-1-certificate-arn>"
release         = ""
```

From the Birdtest checkout:

```sh
terraform -chdir=infra/magpie init
terraform -chdir=infra/magpie plan -var-file=prod.tfvars -out=bootstrap.plan
terraform -chdir=infra/magpie apply bootstrap.plan
terraform -chdir=infra/magpie output dns_cname
```

Review the plan before applying: only the new bucket, distribution and
associated hosting resources should change. Add a DNS CNAME for `magpie` to
the `dns_cname` output (DNS-only if the provider also offers a proxy). The
entry point returns 503 until a release is uploaded and activated.

## 3. Upload, verify, then activate

With `gh`, AWS CLI v2, Python 3 and Terraform installed:

```sh
BUCKET=$(terraform -chdir=infra/magpie output -raw bucket)
python3 scripts/publish-magpie.py wasm-preview-v0.1.0 --bucket "$BUCKET"
```

The script downloads that exact release from `jvc56/MAGPIE`, verifies the
archive and all listed file checksums, and uploads only web assets. It sets
explicit MIME types (especially `.mjs` and `.wasm`) and immutable caching.
The manifest is uploaded last as the completion marker. An interrupted upload
can be rerun: identical objects are skipped, different content is refused,
and conditional writes prevent accidental replacement. It does not activate
the release, delete anything or touch Terraform state.

Before changing the default, test the uploaded version URL with MAGPIE's
browser smoke check (install `wasmentry`'s npm dependencies and Chromium in
that checkout first):

```sh
node wasmentry/qa/release-smoke.mjs https://magpie.birdtest.org/releases/wasm-preview-v0.1.0/wasmentry/
```

Then set `release = "wasm-preview-v0.1.0"` in
`infra/magpie/prod.tfvars`, review and apply:

```sh
terraform -chdir=infra/magpie plan -var-file=prod.tfvars -out=release.plan
terraform -chdir=infra/magpie apply release.plan
```

Terraform refuses activation if the uploaded completion manifest is absent
or names another release. After CloudFront propagation, run the same smoke
check against `https://magpie.birdtest.org/` to check the entry redirect too.
Expect `crossOriginIsolated === true`, successful searches and no isolation
service worker or automatic setup reload.

To roll back, set `release` to a previously uploaded version and apply the
reviewed plan. No rebuild or cache invalidation is needed; already-open tabs
keep their existing release. Keep old releases unless explicitly retiring
their URLs. AWS storage, requests and data transfer incur normal charges.

## Checks without AWS access

```sh
terraform -chdir=infra/magpie init -backend=false
terraform -chdir=infra/magpie validate
terraform -chdir=infra/magpie test
node scripts/magpie-entry-test.cjs
python3 -m unittest discover -s scripts -p test_publish_magpie.py
```

The tests use a mocked AWS provider. CI never applies the infrastructure or
publishes an app version.
