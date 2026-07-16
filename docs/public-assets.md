# Serving Public Assets

Shadow stores objects under content-addressed keys without filename extensions:

```text
<prefix>/<name>/objects/sha256/<first-two-hex>/<remaining-hex>
```

For example, with:

```toml
name = "website"

[backend]
endpoint = "https://tos-cn-beijing.volces.com"
bucket = "example-assets"
prefix = "shadow"
```

an object may have this key:

```text
shadow/website/objects/sha256/ab/cdef...
```

## Content Type

Browsers use the HTTP `Content-Type` response header, not the filename extension, to interpret an object. Shadow determines the media type before publishing and sets it in both normal uploads and multipart upload initialization.

Detection uses this order:

1. a recognizable content signature for specific binary formats;
2. the original worktree path extension for semantic formats such as CSS or JavaScript;
3. `application/octet-stream` when no reliable type is available.

The selected value is derived by the current Shadow version and verified with a remote `HEAD` request. It is not stored in the ref; changes to MIME inference are treated as Shadow behavior changes and `publish` repairs the remote metadata.

## Public URL

TOS object URLs commonly use a virtual-hosted form:

```text
https://<bucket>.<endpoint-host>/<object-key>
```

Using the example above:

```text
https://example-assets.tos-cn-beijing.volces.com/shadow/website/objects/sha256/ab/cdef...
```

The exact public hostname depends on the bucket region, custom-domain configuration, and CDN setup. The API `endpoint` in `shadow.toml` is used for SDK requests and should not be treated as a configurable public asset base URL.

For production websites, prefer a custom domain or CDN:

```text
https://assets.example.com/shadow/website/objects/sha256/ab/cdef...
```

## Access Control

Uploading an object does not automatically make it public. Anonymous reads require an explicit TOS bucket policy or equivalent CDN origin configuration.

Prefer granting read access only to the intended project prefix:

```text
shadow/website/objects/*
```

Do not make a bucket public when it also contains private Shadow repositories. Use a separate bucket or a carefully scoped policy.

Private buckets can use presigned URLs, but those URLs expire and are not suitable as permanent links embedded in a website.

## Caching

Content-addressed URLs are immutable by construction: changing a file changes its SHA-256 and therefore its URL. Shadow sets this default object metadata:

```http
Cache-Control: max-age=31536000, immutable
```

The directive keeps a cached response fresh for one year and tells clients that the URL is immutable. It deliberately omits `public`: anonymous public responses remain cacheable, while authenticated responses are not explicitly opened to shared caches. Bucket access policy remains a separate deployment concern.

Browsers may still evict cached data because of storage pressure or user action. Immutable caching prevents routine revalidation while an entry remains cached; it cannot guarantee permanent local retention.

## URL Discovery

The object ID is stored in `.shadow/refs/<worktree-path>.ref`. Combine that ID with the configured `prefix`, repository `name`, and public hostname to construct the URL.

A future URL-oriented command can automate this presentation-layer mapping without changing the canonical content-addressed object key.
