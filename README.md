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
| CLEARING_CHROMIUM_USER_DATA_DIR                  | clearing user data dir                           | false                                      |
| MONGO_HOST                    | MongoDB host                              | 127.0.0.1                                 |
| MONGO_PORT                    | MongoDB port                              | 27017                                     |
| MONGO_USERNAME                | Username for MongoDB                      | root                                      |
| MONGO_PASSWORD                | Password for MongoDB                      | root                                      |
| MONGO_CONN_TIMEOUT            | MongoDB connection timeout                | N/A                                       |
| MONGO_ADMIN_DATABASE          | Admin database for MongoDB (used to ping) | admin                                     |
| TEMPL_DEFAULT_DATETIME_FORMAT | Default datetime format for templates     | "[day]/[month]/[year] [hour]:[minute]"    |
| TEMPL_DEFAULT_DATE_FORMAT     | Default date format for templates         | "[day]/[month]/[year]"                    |
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

### Pdf rendering

Html templates are printed on A4 with 10mm margins and backgrounds, with the `screen` css media (same defaults as the
backend). A `@page` rule in the template (size, margin) wins over these defaults. Renderings are serialized: the
service drives a single chromium tab.

### dev

```
MONGO_PORT=27018 SERVICE_PORT=8086 SHARE_DRIVE_PATH=/tmp/xx TMP_FS_PATH=/tmp/ouch RUST_LOG=info cargo run -- --generate-openapi
MONGO_PORT=27018 SERVICE_PORT=8086 SHARE_DRIVE_PATH=/tmp/xx TMP_FS_PATH=/tmp/ouch RUST_LOG=info cargo run
```
