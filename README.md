# File Service

Upload file, generate thumbnails and render templates

### Usage

```
services:
  file:
    image: nbittich/file-service
```

### Environment variables

| Environment Variable          | Description                               | Default Value                             |
| ----------------------------- | ----------------------------------------- | ----------------------------------------- |
| BODY_SIZE_LIMIT               | Body size limit for requests (in bytes)   | 1048576 (1mb)                             |
| TMP_FS_PATH                   | tmp fs file path (required!)              | None                                      |
| SERVICE_HOST                  | Hostname of the service                   | 127.0.0.1                                 |
| DEFAULT_TENANT                | Default database name                     | artcoded_test                             |
| SERVICE_PORT                  | Port for the service                      | 80                                        |
| SERVICE_APPLICATION_NAME      | Application name of the service           | file-service                              |
| FILE_SERVICE_COLLECTION_NAME  | Collection name for file service          | fileUpload                                |
| TEMPL_SERVICE_COLLECTION_NAME | Collection name for template service      | template                                  |
| SHARE_DRIVE_PATH              | Path to the shared drive                  | $HOME/file-service or $TMP/file-service   |
| THUMB_HEIGHT                  | Thumbnail height                          | 300                                       |
| THUMB_WIDTH                   | Thumbnail width                           | 300                                       |
| CLEARING_CHROMIUM_USER_DATA_DIR                  | every night: close chromium, clear its user data dir (relaunched on next render) | false                                      |
| MONGO_HOST                    | MongoDB host                              | 127.0.0.1                                 |
| MONGO_PORT                    | MongoDB port                              | 27017                                     |
| MONGO_USERNAME                | Username for MongoDB                      | root                                      |
| MONGO_PASSWORD                | Password for MongoDB                      | root                                      |
| MONGO_CONN_TIMEOUT            | MongoDB connection timeout                | N/A                                       |
| MONGO_ADMIN_DATABASE          | Admin database for MongoDB (used to ping) | admin                                     |
| TEMPL_DEFAULT_DATETIME_FORMAT | Default datetime format (strftime)        | "%d/%m/%Y %H:%M"                          |
| TEMPL_DEFAULT_DATE_FORMAT     | Default date format (strftime)            | "%d/%m/%Y"                                |
| TEMPL_FUEL                    | Max instructions per rendering (0: none)  | 50000000                                  |
| CHROMIUM_SANDBOXED            | Enable Chromium sandboxing                | false                                     |
| PDF_SCALE                     | Scale of the html printed to pdf (0.1-2)  | 1                                         |
| PDF_MEDIA                     | CSS media type used to print (empty: print) | screen                                  |

### Generate openapi client

```
RUST_LOG=info  cargo run -- --generate-openapi
```

```
npm install @openapitools/openapi-generator-cli -g
```

```
openapi-generator-cli generate -i file-service/openapi.json -g java -o file-service-java-client
```

The client version is the `info.version` of the spec (`version` in the `#[openapi(info(...))]` of `src/main.rs`): bump it
when the api changes, then release the client from its repository.

### Templates

Templates are [MiniJinja 3](https://docs.rs/minijinja) (Jinja2) templates, of three types:

| Type   | Rendered as                                                                    |
| ------ | ------------------------------------------------------------------------------ |
| `HTML` | a pdf                                                                          |
| `XML`  | xml (peppol)                                                                   |
| `TEXT` | the text itself, with the content type of the file name: `email.html`, `invite.ics`, `sms.txt` |

A template is compiled when it's uploaded: a syntax error is refused with its line.
`GET /api/v1/template/variables/{id}` lists the variables a template reads (e.g. `invoice.total`).

On top of the Jinja2 builtins:

- `{% include "footer" %}`, `{% extends "email-layout" %}`, `{% from "macros" import x %}`: another template, by
  id or title. Templates can share a layout, a header, macros...
- dates: `{{ d|dateformat }}`, `{{ d|datetimeformat(format="%d/%m/%Y %H:%M", tz="Europe/Brussels") }}`, `now()`.
  Formats are strftime ones (MiniJinja 3; MiniJinja 2 used `[day]/[month]/[year]`, update such templates)
- numbers: `{{ 1234.5|currency }}` (`1.234,50 €`), `{{ x|number(decimals=1, thousands=" ", decimal=".") }}`,
  `{{ x|currency(symbol="$", symbol_first=true) }}`
- `{{ description|nl2br }}`, `{{ data|tojson }}`, `{% break %}` / `{% continue %}`, `loop.previtem` / `loop.nextitem`
- python methods: `{{ name.upper() }}`, `{% for k, v in d.items() %}`, `{{ "{} {}".format(a, b) }}`
- objects keep the key order of the json they come from
- a rendering stops after `TEMPL_FUEL` instructions (runaway loops)

Values are not html-escaped (as before): use `|e` where a value may contain html.

### Pdf rendering

Html templates are printed on A4 with 10mm margins and backgrounds, with the `screen` css media (same defaults as the
backend). A `@page` rule in the template (size, margin) wins over these defaults. Renderings are serialized: the
service drives a single chromium tab.

### dev

```
MONGO_PORT=27018 SERVICE_PORT=8086 SHARE_DRIVE_PATH=/tmp/xx TMP_FS_PATH=/tmp/ouch RUST_LOG=info cargo run -- --generate-openapi
MONGO_PORT=27018 SERVICE_PORT=8086 SHARE_DRIVE_PATH=/tmp/xx TMP_FS_PATH=/tmp/ouch RUST_LOG=info cargo run
```
