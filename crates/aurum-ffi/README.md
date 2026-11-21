# aurum-ffi — мобильное ядро (Swift / Kotlin через UniFFI)

Крейт превращает `aurum-client` (HTTP-клиент) и чистые функции из `aurum-domain`
в библиотеку, которую можно подключить к нативным iOS- и Android-приложениям.
Сами приложения здесь не лежат — только подготовка: API, генерация биндингов,
сборочные скрипты.

## Архитектура

По [ADR 0001](../../docs/adr/0001-rust-workspace-and-ffi-core.md) мобильные
приложения — **тонкие клиенты** self-hosted сервера: никакой локальной БД и
синхронизации, UI нативный (SwiftUI / Jetpack Compose).

```
aurum-domain  ←  aurum-client  ←  aurum-ffi  →  Swift / Kotlin
 (DTO, чистая      (HTTP, без        (UniFFI, async,
  логика, без IO)   состояния)        плоские ошибки)
```

Что экспортируется (proc-macro UniFFI, без UDL):

| Элемент | Назначение |
|---|---|
| `AurumClient(baseUrl, username?, password?, timeoutMs?)` | клиент; `baseUrl` — корень API вместе с `/api` |
| `client.health()` | `GET /health` → `Health { status }` (тот же DTO, что на сервере) |
| `client.request(method, path, query, bodyJson?)` | универсальный вызов любого эндпоинта, ответ — JSON-строка |
| `QueryParam { name, value }` | параметр query (список, порядок и повторы сохраняются) |
| `AurumError` | плоская ошибка: `InvalidArgument`, `Timeout`, `Network`, `Decode`, `Api { status, message, bodyJson }` |
| `contractVersion()`, `decimalQuantize(value, places)` | чистые функции из `aurum-domain` |

Типизированные обёртки добавляются только там, где DTO в `aurum-domain` уже
стабильны (сейчас — `health`). Остальные эндпоинты вызываются через `request`,
а JSON разбирается на стороне приложения.

`request` отправляет `bodyJson` байт-в-байт (после проверки, что это валидный
JSON), поэтому денежные значения вроде `1.10` не теряют формат. Асинхронные
методы работают на tokio-рантайме UniFFI (`async_runtime = "tokio"`): в Swift
это `async throws`, в Kotlin — `suspend`.

### Где живут секреты

Ядро **ничего не хранит и не пишет на диск**. Логин и пароль basic auth
(и адрес сервера) принадлежат платформе:

- iOS — Keychain;
- Android — Android Keystore (например, EncryptedSharedPreferences).

Приложение читает их из хранилища и передаёт в конструктор `AurumClient` при
каждом создании клиента. Пароль в `Debug`-выводе ядра не печатается.

### TLS

`aurum-client` собирается с одной из двух TLS-фич (по умолчанию `tls-aws-lc-rs`,
как у CLI). `aws-lc-rs` тяжело кросс-компилировать под iOS/Android, поэтому
`aurum-ffi` включает фичу `tls-ring` (провайдер `ring`); провайдер ставится
автоматически при создании клиента.

> Android: rustls по умолчанию проверяет сертификаты через платформенный
> верификатор (`rustls-platform-verifier`), которому на Android нужна
> инициализация со стороны JVM (см. документацию крейта
> `rustls-platform-verifier`). На устройстве это ещё не проверялось — задача
> этапа с самим приложением. Для `http://` (локальная сеть, VPN) TLS не нужен.

## Генерация биндингов

Нужны только Rust (rustup) и cargo:

```sh
crates/aurum-ffi/scripts/gen-bindings.sh            # debug
crates/aurum-ffi/scripts/gen-bindings.sh --release
```

Скрипт собирает хостовую библиотеку и запускает `uniffi-bindgen` для Swift и
Kotlin. Результат (в `.gitignore`):

```
crates/aurum-ffi/bindings/swift/   aurum_ffi.swift, aurum_ffiFFI.h, aurum_ffiFFI.modulemap
crates/aurum-ffi/bindings/kotlin/  uniffi/aurum_ffi/aurum_ffi.kt
```

То же вручную:

```sh
cargo build -p aurum-ffi
cargo run -p aurum-ffi --bin uniffi-bindgen generate \
  --library target/debug/libaurum_ffi.so --language swift --out-dir out/swift
```

(на macOS расширение `.dylib`). Если установлены `swiftformat`/`ktlint`,
сгенерированный код будет отформатирован автоматически.

## Сборка под iOS

Только macOS с Xcode:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
crates/aurum-ffi/scripts/build-ios.sh            # release; --debug для отладки
```

Результат — `crates/aurum-ffi/build/ios/`:

- `AurumFFI.xcframework` — слайсы устройства (arm64) и симулятора (arm64 + x86_64);
- `swift/aurum_ffi.swift` — Swift-обёртка.

На другой ОС скрипт сразу завершается с понятной ошибкой.

### Подключение в Xcode / SwiftPM

Xcode: перетащите `AurumFFI.xcframework` в *Frameworks, Libraries, and Embedded
Content* (Do Not Embed — библиотека статическая) и добавьте `aurum_ffi.swift` в
таргет.

SwiftPM (`Package.swift`):

```swift
let package = Package(
    name: "AurumCore",
    platforms: [.iOS(.v15)],
    products: [.library(name: "AurumCore", targets: ["AurumCore"])],
    targets: [
        .binaryTarget(name: "AurumFFI", path: "AurumFFI.xcframework"),
        // aurum_ffi.swift лежит в Sources/AurumCore/
        .target(name: "AurumCore", dependencies: ["AurumFFI"]),
    ]
)
```

### Пример на Swift

```swift
import Foundation

// Креды берутся из Keychain; ядро их не хранит.
let client = try AurumClient(
    baseUrl: "https://aurum.example.com/api",
    username: keychain.username,
    password: keychain.password,
    timeoutMs: 15_000
)

do {
    let health = try await client.health()
    print(health.status)

    let json = try await client.request(
        method: "GET",
        path: "/accounts",
        query: [QueryParam(name: "limit", value: "50")],
        bodyJson: nil
    )
    // json — строка с JSON, разбирается через JSONDecoder на стороне приложения
    print(decimalQuantize(value: "1.005", places: 2)) // "1.00"
} catch let AurumError.api(status, message, _) {
    print("HTTP \(status): \(message)")
} catch {
    print(error)
}
```

## Сборка под Android

Нужны Android NDK (`ANDROID_NDK_HOME`) и [cargo-ndk](https://github.com/bbqsrc/cargo-ndk):

```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
crates/aurum-ffi/scripts/build-android.sh        # release; --debug для отладки
```

Результат — `crates/aurum-ffi/build/android/`:

- `jniLibs/{arm64-v8a,armeabi-v7a,x86_64}/libaurum_ffi.so`;
- `kotlin/uniffi/aurum_ffi/aurum_ffi.kt`.

### Подключение в Gradle

```
app/src/main/jniLibs/        ← содержимое build/android/jniLibs/
app/src/main/java/uniffi/aurum_ffi/aurum_ffi.kt   ← build/android/kotlin/...
```

Kotlin-биндинги используют JNA, добавьте зависимость (`build.gradle.kts`):

```kotlin
dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")
}
```

Или укажите папки в `sourceSets` (`jniLibs.srcDir("../crates/aurum-ffi/build/android/jniLibs")`).

### Пример на Kotlin

```kotlin
import uniffi.aurum_ffi.AurumClient
import uniffi.aurum_ffi.AurumException
import uniffi.aurum_ffi.QueryParam

// Креды берутся из Android Keystore; ядро их не хранит.
val client = AurumClient(
    baseUrl = "https://aurum.example.com/api",
    username = secrets.username,
    password = secrets.password,
    timeoutMs = 15_000u,
)

suspend fun load(): String? = try {
    println(client.health().status)
    client.request("GET", "/accounts", listOf(QueryParam("limit", "50")), null)
} catch (e: AurumException.Api) {
    println("HTTP ${e.status}: ${e.message}")
    null
}
```

## Проверка и CI

```sh
cargo fmt -p aurum-ffi --check
cargo clippy -p aurum-ffi -p aurum-client --all-targets --no-deps -- -D warnings
cargo test -p aurum-ffi -p aurum-client   # интеграционные тесты против локального mock-сервера
```

Job `ffi-tests` в `.github/workflows/ci.yml` делает то же самое на Ubuntu и
дополнительно генерирует биндинги. Кросс-сборки под iOS/Android в CI не
запускаются (нужны macOS и NDK) — их выполняют скрипты локально.
