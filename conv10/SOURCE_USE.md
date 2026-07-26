# Source-use audit

Audited: 2026-07-26 UTC

The two checked-in lists contain 96 source recordings. The source pages showed:

| List | CC0 / public domain | CC BY | Commercially usable | Rejected |
|---|---:|---:|---:|---:|
| conv10 | 29 | 19 | 48 | 0 |
| conv8 | 29 | 19 | 48 | 0 |
| Total | 58 | 38 | 96 | 0 |

The audit accepts CC0/public-domain material and plain CC BY material. It rejects
NC, ND, SA, Sampling+, unsupported, and unclear terms. No source needed
replacement. CC BY sources require attribution; the ready-to-paste text is in
`YOUTUBE_ATTRIBUTION.md`.

`LICENSE_AUDIT_CONV10.tsv` and `LICENSE_AUDIT_CONV8.tsv` record the creator,
source page, displayed license, canonical license URL, decision, retrieval time,
and a SHA-256 hash of the retrieved source-page HTML. Rerun the evidence check:

```bash
./scripts/audit_licenses.py \
  --manifest sources.tsv \
  --output LICENSE_AUDIT_CONV10.tsv

./scripts/audit_licenses.py \
  --manifest LICENSE_AUDIT_CONV8.tsv \
  --output LICENSE_AUDIT_CONV8.tsv

./scripts/write_attribution.py
```

Commercial permission is not a promise that YouTube will never issue a Content
ID claim or make a separate channel-monetization decision. Freesound documents
that shared source effects can cause Content ID matches even when reuse is
licensed. Keep the audit tables and attribution text, upload privately first,
let YouTube complete its checks, and dispute any mistaken claim with the source
page and license evidence.
