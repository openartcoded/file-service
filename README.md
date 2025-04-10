# Kofte

Upload file, generate thumbnails and render templates

### Usage

```
services:
  kofte:
    image: oxfamily/kofte
```

### Environment variables

| Environment Variable          | Description                               | Default Value                             |
| ----------------------------- | ----------------------------------------- | ----------------------------------------- |
| BODY_SIZE_LIMIT               | Body size limit for requests (in bytes)   | 1048576 (1mb)                             |
| SERVICE_HOST                  | Hostname of the service                   | 127.0.0.1                                 |
| SERVICE_PORT                  | Port for the service                      | 80                                        |
| SERVICE_APPLICATION_NAME      | Application name of the service           | kofte-service                             |
| FILE_SERVICE_COLLECTION_NAME  | Collection name for file service          | fileUpload                                |
| TEMPL_SERVICE_COLLECTION_NAME | Collection name for template service      | template                                  |
| SHARE_DRIVE_PATH              | Path to the shared drive                  | $HOME/kofte-service or $TMP/kofte-service |
| THUMB_HEIGHT                  | Thumbnail height                          | 300                                       |
| THUMB_WIDTH                   | Thumbnail width                           | 300                                       |
| TZ                            | Time zone                                 | Europe/Brussels                           |
| MONGO_HOST                    | MongoDB host                              | 127.0.0.1                                 |
| MONGO_PORT                    | MongoDB port                              | 27017                                     |
| MONGO_USERNAME                | Username for MongoDB                      | root                                      |
| MONGO_PASSWORD                | Password for MongoDB                      | root                                      |
| MONGO_CONN_TIMEOUT            | MongoDB connection timeout                | N/A                                       |
| MONGO_ADMIN_DATABASE          | Admin database for MongoDB (used to ping) | admin                                     |
| TEMPL_DEFAULT_DATETIME_FORMAT | Default datetime format for templates     | "[day]/[month]/[year] [hour]:[minute]"    |
| TEMPL_DEFAULT_DATE_FORMAT     | Default date format for templates         | "[day]/[month]/[year]"                    |
| CHROMIUM_SANDBOXED            | Enable Chromium sandboxing                | false                                     |

### Generate openapi client

```
RUST_LOG=info  cargo run -- --generate-openapi
```

```
npm install @openapitools/openapi-generator-cli -g
```

```
openapi-generator-cli generate -i kofte-rs/openapi.json -g java -o kofte-java-client
```
