# Quy trinh build va deploy BotBanHang

Tai lieu nay ghi lai quy trinh build va deploy len VPS Linux cho project `botbanhang`.

VPS dang dung trong du an:

```text
ipvps
```

Service tren VPS:

```text
botbanhang
```

Thu muc cai dat tren VPS:

```text
/opt/botbanhang
```

## 1. Tong quan luong deploy

Quy trinh deploy gom cac buoc:

1. Kiem tra code local.
2. Chay format/test neu can.
3. Build binary Linux bang Docker tren may Windows.
4. Dong goi binary, `deploy.sh`, `public/`, `i18n/`, va `scripts/`.
5. Upload file `.tar.gz` len VPS.
6. Giai nen tren VPS.
7. Chay `deploy.sh`; script se stop service, backup binary/DB hien tai, roi moi copy binary moi vao `/opt/botbanhang`.
8. Restart systemd service.
9. Kiem tra service, log, va web admin.

Neu project nay nam trong repo `bot-manager`, flow nhanh nen chay tu repo cha:

```powershell
cd C:\Users\NAM\Code\rust\bot-manager
$env:BOT_MANAGER_ARTIFACT_SIGNATURE_SECRET = "<artifact-signature-secret>"
.\scripts\deploy.ps1 -VpsHost root@157.230.243.74
```

Lenh tren se build artifact Linux cua `botbanhang`, copy vao `artifacts/`, build va deploy `bot-manager`. Chi dung quy trinh deploy rieng trong tai lieu nay khi can cap nhat service `botbanhang` doc lap o `/opt/botbanhang`.

Khong build truc tiep bang `build.ps1` neu muc tieu la VPS Linux, vi `build.ps1` hien tai build binary Windows native:

```powershell
cargo build --release
```

Binary Windows nam o:

```text
target/release/botbanhang.exe
```

VPS can binary Linux:

```text
botbanhang
```

## 2. Yeu cau tren may local

May local can co:

```powershell
docker --version
ssh -V
scp
cargo --version
```

Kiem tra Docker:

```powershell
docker --version
```

Kiem tra SSH vao VPS:

```powershell
ssh -o BatchMode=yes -o ConnectTimeout=10 root@ipvps "uname -m && systemctl is-active botbanhang || true"
```

Ket qua mong muon:

```text
x86_64
active
```

Neu SSH hoi password/key thi can dam bao may local da co SSH key hop le de vao VPS.

## 3. Kiem tra nhanh truoc khi build

Tu thu muc repo:

```powershell
cd C:\Users\NAM\Code\rust\botbanhang2
```

Kiem tra thay doi hien tai:

```powershell
git status --short
```

Chay format:

```powershell
cargo fmt
```

Chay test:

```powershell
cargo test
```

Neu chi muon test nhom lien quan file upload:

```powershell
cargo test uploaded_file
```

Chi deploy khi build/test khong loi.

## 4. Kiem tra GLIBC tren VPS

Binary Linux build tu Docker phai tuong thich voi GLIBC tren VPS.

Kiem tra GLIBC VPS:

```powershell
ssh root@ipvps "ldd --version | head -1"
```

Vi du ket qua hien tai:

```text
ldd (Ubuntu GLIBC 2.39-0ubuntu8.7) 2.39
```

Nen build bang Docker image co GLIBC bang hoac thap hon VPS. Image da dung on dinh:

```text
rust:1.88-bookworm
```

Image nay dung Debian bookworm GLIBC `2.36`, chay duoc tren VPS GLIBC `2.39`.

Khong nen build bang image qua moi neu VPS co GLIBC cu hon. Vi du binary build tren GLIBC `2.41` co the khong chay tren VPS GLIBC `2.39`.

## 5. Build Linux binary bang Docker

Dung target rieng cho Docker deploy:

```text
target-docker-deploy
```

Lenh build:

```powershell
docker run --rm `
  -v "${PWD}:/app" `
  -w /app `
  -e CARGO_TARGET_DIR=/app/target-docker-deploy `
  rust:1.88-bookworm `
  bash -lc "export PATH=/usr/local/cargo/bin:/usr/local/rustup/toolchains/1.88.0-x86_64-unknown-linux-gnu/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin; cargo build --release"
```

Sau khi build xong, binary Linux nam o:

```text
target-docker-deploy/release/botbanhang
```

Kiem tra file ton tai:

```powershell
Get-Item target-docker-deploy\release\botbanhang
```

## 6. Tao goi deploy

Goi deploy can gom:

```text
botbanhang
deploy.sh
public/
i18n/
scripts/
```

Tao thu muc staging va nen thanh `.tar.gz`:

```powershell
$staging = Join-Path (Get-Location) 'dist-deploy-current'

if (Test-Path -LiteralPath $staging) {
    $resolved = (Resolve-Path -LiteralPath $staging).Path
    $root = (Resolve-Path -LiteralPath (Get-Location)).Path
    if (-not $resolved.StartsWith($root)) {
        throw "Refusing to remove outside workspace: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

New-Item -ItemType Directory -Path $staging | Out-Null
Copy-Item -LiteralPath 'target-docker-deploy\release\botbanhang' -Destination (Join-Path $staging 'botbanhang') -Force
Copy-Item -LiteralPath 'deploy.sh' -Destination (Join-Path $staging 'deploy.sh') -Force
Copy-Item -LiteralPath 'public' -Destination (Join-Path $staging 'public') -Recurse -Force
Copy-Item -LiteralPath 'i18n' -Destination (Join-Path $staging 'i18n') -Recurse -Force
Copy-Item -LiteralPath 'scripts' -Destination (Join-Path $staging 'scripts') -Recurse -Force

if (Test-Path -LiteralPath 'botbanhang-deploy.tar.gz') {
    Remove-Item -LiteralPath 'botbanhang-deploy.tar.gz' -Force
}

tar -czf botbanhang-deploy.tar.gz -C dist-deploy-current .
Get-Item botbanhang-deploy.tar.gz
```

File tao ra:

```text
botbanhang-deploy.tar.gz
```

## 7. Upload len VPS

Upload goi deploy vao `/tmp`:

```powershell
scp botbanhang-deploy.tar.gz root@ipvps:/tmp/botbanhang-deploy.tar.gz
```

Neu upload thanh cong lenh se thoat khong bao loi.

## 8. Deploy tren VPS

Chay lenh SSH sau de giai nen va deploy:

```powershell
ssh root@ipvps "rm -rf /tmp/botbanhang-deploy && mkdir -p /tmp/botbanhang-deploy && tar -xzf /tmp/botbanhang-deploy.tar.gz -C /tmp/botbanhang-deploy && cd /tmp/botbanhang-deploy && chmod +x botbanhang deploy.sh && BIN_PATH=./botbanhang bash ./deploy.sh && systemctl is-active botbanhang && systemctl status botbanhang --no-pager -l | sed -n '1,12p'"
```

`deploy.sh` se lam cac viec sau:

1. Tao `/opt/botbanhang`.
2. Tao `/opt/botbanhang/storage/uploads`.
3. Tao `/opt/botbanhang/storage/product_files`.
4. Stop service `botbanhang` neu dang chay.
5. Tao thu muc backup `/opt/botbanhang/backups`.
6. Backup binary hien tai vao `/opt/botbanhang/backups/botbanhang.YYYYMMDDHHMMSS`.
7. Doc `DATABASE_URL` tu `/opt/botbanhang/.env`, mac dinh la `sqlite://shop.db` neu khong co.
8. Backup DB SQLite vao `/opt/botbanhang/backups/db-YYYYMMDDHHMMSS/`.
9. Copy binary moi vao `/opt/botbanhang/botbanhang`.
10. Copy `.env` neu trong goi deploy co `.env`.
11. Copy `public/` vao `/opt/botbanhang/public`.
12. Merge `i18n/` vao `/opt/botbanhang/i18n`: giu gia tri runtime da sua, chi them language/file/key con thieu tu artifact. Buoc merge dung chinh binary `botbanhang`, khong can cai Python tren VPS.
13. Ghi systemd service vao `/etc/systemd/system/botbanhang.service`.
14. Reload systemd.
15. Enable va start service.

Vi service da stop truoc khi backup DB, cac file SQLite duoc copy o trang thai on dinh:

```text
shop.db
shop.db-wal
shop.db-shm
```

Neu `shop.db-wal` hoac `shop.db-shm` khong ton tai thi script bo qua file do.

Sau deploy, kiem tra backup moi nhat:

```powershell
ssh root@ipvps "ls -lah /opt/botbanhang/backups | tail -20"
```

Luu y: Goi deploy mac dinh o buoc tren khong copy `.env`. Neu can cap nhat `.env`, copy rieng hoac them `.env` vao staging co chu y. Khong nen vo tinh upload `.env` neu dang chua secret local khac production.

## 9. Kiem tra sau deploy

Kiem tra service:

```powershell
ssh root@ipvps "systemctl is-active botbanhang"
```

Ket qua mong muon:

```text
active
```

Kiem tra status:

```powershell
ssh root@ipvps "systemctl status botbanhang --no-pager -l | sed -n '1,16p'"
```

Kiem tra log gan nhat:

```powershell
ssh root@ipvps "journalctl -u botbanhang -n 50 --no-pager"
```

Nhung dong log mong muon:

```text
Successfully registered bot commands to Telegram
Web server listening on 0.0.0.0:8080
Bot started as @...
```

Kiem tra web admin tren local VPS:

```powershell
ssh root@ipvps "curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/admin"
```

Ket qua mong muon:

```text
200
```

## 10. Kiem tra nhanh tinh nang sau deploy

Sau khi deploy, nen test bang Telegram:

1. Gui `/start`.
2. Gui `/shop`.
3. Chon san pham binh thuong.
4. Chon san pham file upload.
5. Kiem tra san pham file upload co man nhap so luong.
6. Tao don test.
7. Neu khong thanh toan, doi qua thoi gian het han de kiem tra tra hang.
8. Neu thanh toan test, kiem tra bot gui dung file da giu.

Neu can kiem tra worker tra don qua han:

```powershell
ssh root@ipvps "journalctl -u botbanhang -f"
```

Tim log:

```text
pending cleanup tick finished
```

## 11. Bao mat file upload ban hang

File san pham ban hang nam trong:

```text
/opt/botbanhang/storage/product_files
```

Thu muc nay khong duoc expose ra web. Code chi serve public upload tai:

```text
/uploads -> storage/uploads
```

Kiem tra HTTP:

```powershell
ssh root@ipvps "curl -sS -o /dev/null -w '/storage/product_files => %{http_code}\n' http://127.0.0.1:8080/storage/product_files/; curl -sS -o /dev/null -w '/product_files => %{http_code}\n' http://127.0.0.1:8080/product_files/"
```

Ket qua mong muon:

```text
/storage/product_files => 404
/product_files => 404
```

Siet quyen file san pham:

```powershell
ssh root@ipvps "chmod 700 /opt/botbanhang/storage/product_files && find /opt/botbanhang/storage/product_files -type f -exec chmod 600 {} +"
```

Kiem tra quyen:

```powershell
ssh root@ipvps "find /opt/botbanhang/storage/product_files -maxdepth 1 -printf '%M %u:%g %p\n' | sed -n '1,20p'"
```

Ket qua mong muon:

```text
drwx------ root:root /opt/botbanhang/storage/product_files
-rw------- root:root /opt/botbanhang/storage/product_files/...
```

## 12. Don rac local sau deploy

Cac thu muc/file build artifact co the xoa sau khi deploy thanh cong:

```text
target-docker-deploy/
dist-deploy-current/
botbanhang-deploy.tar.gz
```

Lenh xoa:

```powershell
$root = (Resolve-Path -LiteralPath (Get-Location)).Path
$paths = @('target-docker-deploy', 'dist-deploy-current', 'botbanhang-deploy.tar.gz')

foreach ($name in $paths) {
    $path = Join-Path $root $name
    if (Test-Path -LiteralPath $path) {
        $resolved = (Resolve-Path -LiteralPath $path).Path
        if (-not $resolved.StartsWith($root)) {
            throw "Refusing to remove outside workspace: $resolved"
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
```

Khong xoa:

```text
target/
shop.db
shop.db-shm
shop.db-wal
.env
public/
i18n/
scripts/
src/
migrations/
```

## 13. Lenh deploy nhanh mot lan

Neu da hieu ro quy trinh, co the chay day du theo thu tu nay:

```powershell
cd C:\Users\NAM\Code\rust\botbanhang2

cargo fmt
cargo test

docker run --rm `
  -v "${PWD}:/app" `
  -w /app `
  -e CARGO_TARGET_DIR=/app/target-docker-deploy `
  rust:1.88-bookworm `
  bash -lc "export PATH=/usr/local/cargo/bin:/usr/local/rustup/toolchains/1.88.0-x86_64-unknown-linux-gnu/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin; cargo build --release"

$staging = Join-Path (Get-Location) 'dist-deploy-current'
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging | Out-Null
Copy-Item -LiteralPath 'target-docker-deploy\release\botbanhang' -Destination (Join-Path $staging 'botbanhang') -Force
Copy-Item -LiteralPath 'deploy.sh' -Destination (Join-Path $staging 'deploy.sh') -Force
Copy-Item -LiteralPath 'public' -Destination (Join-Path $staging 'public') -Recurse -Force
Copy-Item -LiteralPath 'i18n' -Destination (Join-Path $staging 'i18n') -Recurse -Force
Copy-Item -LiteralPath 'scripts' -Destination (Join-Path $staging 'scripts') -Recurse -Force
if (Test-Path -LiteralPath 'botbanhang-deploy.tar.gz') { Remove-Item -LiteralPath 'botbanhang-deploy.tar.gz' -Force }
tar -czf botbanhang-deploy.tar.gz -C dist-deploy-current .

scp botbanhang-deploy.tar.gz root@ipvps:/tmp/botbanhang-deploy.tar.gz

ssh root@ipvps "rm -rf /tmp/botbanhang-deploy && mkdir -p /tmp/botbanhang-deploy && tar -xzf /tmp/botbanhang-deploy.tar.gz -C /tmp/botbanhang-deploy && cd /tmp/botbanhang-deploy && chmod +x botbanhang deploy.sh && BIN_PATH=./botbanhang bash ./deploy.sh && systemctl is-active botbanhang && systemctl status botbanhang --no-pager -l | sed -n '1,12p'"

ssh root@ipvps "curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/admin"
```

## 14. Loi thuong gap

### `cargo: command not found` trong Docker

Neu Docker image co PATH khong dung, dung lenh co `export PATH=...` nhu tai lieu nay.

### Loi GLIBC

Vi du:

```text
version `GLIBC_2.39' not found
```

Nguyen nhan: artifact cu hoac binary build bang moi truong GLIBC cao hon moi truong dang chay.

Cach xu ly:

1. Xoa target Docker cu.
2. Build bang image GLIBC thap hon hoac bang VPS.

Lenh xoa:

```powershell
Remove-Item -LiteralPath target-docker-deploy -Recurse -Force
```

Sau do build lai bang:

```text
rust:1.88-bookworm
```

### Service khong active

Kiem tra log:

```powershell
ssh root@ipvps "journalctl -u botbanhang -n 100 --no-pager"
```

Cac loi can xem:

```text
missing .env
DATABASE_URL loi
PORT da bi chiem
TELOXIDE_TOKEN sai
permission denied khi doc file
```

### Admin tra 500 hoac 404

Kiem tra `public/` da duoc copy chua:

```powershell
ssh root@ipvps "ls -la /opt/botbanhang/public"
```

Kiem tra endpoint:

```powershell
ssh root@ipvps "curl -i http://127.0.0.1:8080/admin | head"
```

### Bot khong phan hoi Telegram

Kiem tra log:

```powershell
ssh root@ipvps "journalctl -u botbanhang -n 100 --no-pager"
```

Kiem tra token trong `.env` tren VPS:

```powershell
ssh root@ipvps "sudo grep -E '^TELOXIDE_TOKEN=' /opt/botbanhang/.env | sed 's/=.*/=***hidden***/'"
```

Restart service:

```powershell
ssh root@ipvps "systemctl restart botbanhang && systemctl is-active botbanhang"
```

## 15. Rollback nhanh

`deploy.sh` da tu dong backup binary va DB truoc khi thay binary moi.

Thu muc backup:

```text
/opt/botbanhang/backups
```

Kiem tra cac ban backup:

```powershell
ssh root@ipvps "ls -lah /opt/botbanhang/backups"
```

### Rollback binary

Chon file binary backup dang co dang:

```text
/opt/botbanhang/backups/botbanhang.YYYYMMDDHHMMSS
```

Thay lai binary:

```powershell
ssh root@ipvps "systemctl stop botbanhang && cp /opt/botbanhang/backups/botbanhang.YYYYMMDDHHMMSS /opt/botbanhang/botbanhang && chmod +x /opt/botbanhang/botbanhang && systemctl start botbanhang && systemctl is-active botbanhang"
```

### Rollback DB

Chi rollback DB khi chac chan can quay lai du lieu cu, vi rollback DB se mat cac don/user/thay doi phat sinh sau thoi diem backup.

Chon thu muc DB backup:

```text
/opt/botbanhang/backups/db-YYYYMMDDHHMMSS/
```

Stop service va copy DB backup ve lai:

```powershell
ssh root@ipvps "systemctl stop botbanhang && cp /opt/botbanhang/backups/db-YYYYMMDDHHMMSS/shop.db /opt/botbanhang/shop.db && if [ -f /opt/botbanhang/backups/db-YYYYMMDDHHMMSS/shop.db-wal ]; then cp /opt/botbanhang/backups/db-YYYYMMDDHHMMSS/shop.db-wal /opt/botbanhang/shop.db-wal; else rm -f /opt/botbanhang/shop.db-wal; fi && if [ -f /opt/botbanhang/backups/db-YYYYMMDDHHMMSS/shop.db-shm ]; then cp /opt/botbanhang/backups/db-YYYYMMDDHHMMSS/shop.db-shm /opt/botbanhang/shop.db-shm; else rm -f /opt/botbanhang/shop.db-shm; fi && systemctl start botbanhang && systemctl is-active botbanhang"
```

Neu `DATABASE_URL` tren VPS khong phai `sqlite://shop.db`, thay `/opt/botbanhang/shop.db` bang dung duong dan DB production.

### Backup thu cong ngoai deploy

Neu muon backup rieng ma khong deploy:

```powershell
ssh root@ipvps "systemctl stop botbanhang && ts=$(date -u +%Y%m%d%H%M%S) && mkdir -p /opt/botbanhang/backups/db-$ts && cp -a /opt/botbanhang/botbanhang /opt/botbanhang/backups/botbanhang.$ts && cp -a /opt/botbanhang/shop.db /opt/botbanhang/backups/db-$ts/ && [ ! -f /opt/botbanhang/shop.db-wal ] || cp -a /opt/botbanhang/shop.db-wal /opt/botbanhang/backups/db-$ts/ && [ ! -f /opt/botbanhang/shop.db-shm ] || cp -a /opt/botbanhang/shop.db-shm /opt/botbanhang/backups/db-$ts/ && systemctl start botbanhang && systemctl is-active botbanhang"
```

## 16. Ghi chu ve du lieu

Khong copy de len cac file du lieu production neu khong co chu y:

```text
/opt/botbanhang/shop.db
/opt/botbanhang/shop.db-shm
/opt/botbanhang/shop.db-wal
/opt/botbanhang/storage/uploads
/opt/botbanhang/storage/product_files
/opt/botbanhang/.env
```

`deploy.sh` backup DB va binary truoc khi thay binary moi. Sau do script copy binary, `public/`, va merge `i18n/` theo kieu giu gia tri runtime da sua, chi them key con thieu. No tao thu muc storage neu chua co, nhung khong xoa `storage/product_files`.

Dieu nay giup file san pham da upload va DB production khong bi mat khi deploy lai.
