# Tai lieu trang Admin va API

Tai lieu nay duoc tong hop tu `src/core/pages.rs`, `src/domains/*/api.rs` va `public/admin.html`.

## Tong quan

- Trang quan tri duoc serve tai `/admin` tu file `public/admin.html`.
- API dang nhap dung prefix `/api/auth/*`.
- API quan tri dung prefix `/api/admin/*`.
- Cac API `/api/admin/*` duoc bao ve bang cookie session `admin_session` qua middleware `require_admin_session`.
- Response JSON chuan cua backend:

```json
{
  "ok": true,
  "data": {}
}
```

Khi loi:

```json
{
  "ok": false,
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "noi dung loi"
  }
}
```

## Luong xac thuc admin

Trang admin goi cac API sau:

| Method | Endpoint | Muc dich |
| --- | --- | --- |
| `GET` | `/api/auth/setup-status` | Kiem tra he thong da co admin dau tien chua. |
| `POST` | `/api/auth/setup` | Tao admin dau tien khi chua setup. |
| `POST` | `/api/auth/login` | Dang nhap admin, server set cookie `admin_session`. |
| `POST` | `/api/auth/logout` | Dang xuat va xoa cookie session. |
| `GET` | `/api/auth/me` | Lay thong tin admin dang dang nhap. |

Payload setup:

```json
{
  "username": "admin",
  "password": "password",
  "setup_code": "ADMIN_SETUP_CODE"
}
```

Payload login:

```json
{
  "username": "admin",
  "password": "password"
}
```

Session hien tai het han sau 8 gio. Cac request tu trang admin gui `credentials: "same-origin"` de cookie duoc truyen kem.

## API dung chung tren trang admin

| Method | Endpoint | Muc dich |
| --- | --- | --- |
| `GET` | `/api/health` | Nut Ping tren header, kiem tra server con song. |
| `GET` | `/ping` | Health text don gian tra ve `pong`. |

## Tab San pham

Chuc nang tren UI:

- Xem danh sach san pham, tim theo ten, loc theo trang thai ban.
- Tao/sua san pham.
- Upload/xoa anh san pham.
- Upload/xoa file giao hang co dinh.
- Bat/tat ban san pham.
- Sap xep san pham bang nut len/xuong hoac keo tha.
- Quan ly kho item text va goi/thang cua san pham.
- Xem doanh thu 7 ngay va 30 ngay.

API:

| Method | Endpoint | Query/Payload chinh |
| --- | --- | --- |
| `GET` | `/api/admin/products` | `limit`, `offset`, `active=all/1/0`, `query` |
| `POST` | `/api/admin/products` | `ProductPayload` |
| `GET` | `/api/admin/products/:id` | Lay chi tiet san pham. |
| `PUT` | `/api/admin/products/:id` | `ProductPayload` |
| `DELETE` | `/api/admin/products/:id` | Disable/ngung ban san pham. |
| `POST` | `/api/admin/products/:id/toggle` | `{ "is_active": 1 }` hoac `{ "is_active": 0 }` |
| `POST` | `/api/admin/products/reorder` | `{ "items": [{ "id": 1, "sort_order": 10 }] }` |
| `POST` | `/api/admin/products/:id/image` | Multipart upload field `file`. |
| `DELETE` | `/api/admin/products/:id/image` | Xoa anh hien tai. |
| `POST` | `/api/admin/products/:id/file` | Multipart upload field `file`. |
| `DELETE` | `/api/admin/products/:id/file` | Xoa file giao hang co dinh. |
| `GET` | `/api/admin/products/:id/stock` | Lay so luong item ton. |
| `GET` | `/api/admin/products/:id/items` | `limit`, `offset` |
| `POST` | `/api/admin/products/:id/items` | `{ "items": ["dong 1", "dong 2"] }` |
| `DELETE` | `/api/admin/products/:id/items/:item_id` | Xoa mot item kho. |
| `GET` | `/api/admin/products/:id/plans` | Lay danh sach goi. |
| `POST` | `/api/admin/products/:id/plans` | `{ "label": "1 thang", "months": 1, "price": 100000, "sort_order": 0 }` |
| `PUT` | `/api/admin/products/:id/plans/:plan_id` | Cap nhat goi. |
| `DELETE` | `/api/admin/products/:id/plans/:plan_id` | Xoa goi. |
| `GET` | `/api/admin/stats/revenue?days=7` | Tong doanh thu trong N ngay. |

`ProductPayload`:

```json
{
  "name": "Ten san pham",
  "price": 100000,
  "is_active": 1,
  "requires_input": 0,
  "input_prompt": "Nhap email kich hoat",
  "description": "Mo ta",
  "image_url": "/uploads/products/image.jpg",
  "delivery_type": "text",
  "file_path": "/uploads/products/file.zip",
  "file_name": "file.zip",
  "file_mime": "application/zip"
}
```

`delivery_type` duoc UI dung de phan biet cach giao hang, gom cac nhom chinh: item text trong kho, san pham can thong tin kich hoat, hoac file upload co dinh.

## Tab Don hang

Chuc nang tren UI:

- Xem danh sach don hang.
- Loc theo trang thai, tu khoa, ngay bat dau, ngay ket thuc.
- Xem chi tiet don.
- Danh dau da thanh toan thu cong.
- Gui lai du lieu giao hang.
- Huy don hang.
- Xuat file CSV.

API:

| Method | Endpoint | Query/Payload chinh |
| --- | --- | --- |
| `GET` | `/api/admin/orders` | `status`, `limit`, `offset`, `query`, `from`, `to` |
| `GET` | `/api/admin/orders/export` | Cung bo loc voi danh sach, tra ve file export. |
| `GET` | `/api/admin/orders/:id` | Lay chi tiet don va san pham. |
| `POST` | `/api/admin/orders/:id/mark_paid` | `{ "payment_tx_id": "...", "paid_at": "2026-04-13T10:20:30Z" }` |
| `POST` | `/api/admin/orders/:id/cancel` | `{ "reason": "admin" }` |
| `POST` | `/api/admin/orders/:id/resend` | Gui lai delivered data/file cho khach. |

Trang thai don hang:

- `pending`: Cho thanh toan.
- `paid`: Da thanh toan.
- `cancel`: Da huy.
- `expired`: Het han.

## Tab Vi tien

Chuc nang tren UI:

- Xem danh sach user co vi.
- Tim theo user id, chat id, username, ten.
- Xem so du va 20 giao dich gan nhat.
- Nap tien thu cong theo user id.
- Dieu chinh so du thu cong.

API:

| Method | Endpoint | Query/Payload chinh |
| --- | --- | --- |
| `GET` | `/api/admin/wallets` | `limit`, `offset`, `query` |
| `GET` | `/api/admin/wallets/:user_id` | Lay `wallet` va `transactions`. |
| `POST` | `/api/admin/wallets/:user_id/topup` | `{ "amount": 100000, "note": "Nap tay", "setup_code": "..." }` |
| `POST` | `/api/admin/wallets/:user_id/adjust` | `{ "amount": -50000, "note": "Dieu chinh", "setup_code": "..." }` |

Luu y:

- `topup` yeu cau `amount > 0`.
- `adjust` cho phep cong hoac tru, nhung `amount` khong duoc bang 0.
- `note` toi da 500 ky tu.
- Hai API thay doi so du deu yeu cau `setup_code`.

## Tab Thong bao

Chuc nang tren UI:

- Gui broadcast message toi user.
- Tuy chon preview/dry-run tuy theo payload UI gui len.

API:

| Method | Endpoint | Muc dich |
| --- | --- | --- |
| `POST` | `/api/admin/broadcast` | Gui thong bao hang loat. |

Payload duoc gui tu trang admin gom noi dung thong bao va cac tuy chon gui theo form broadcast. Handler nam tai `src/domains/users/broadcast.rs`.

## Tab Webhook

Chuc nang tren UI:

- Xem log webhook thanh toan.
- Loc theo provider, memo, transaction id.

API:

| Method | Endpoint | Query |
| --- | --- | --- |
| `GET` | `/api/admin/webhooks/events` | `limit`, `offset`, `provider`, `memo`, `tx_id` |

Du lieu tra ve theo dang phan trang, item co cac thong tin chinh: `received_at`, `provider`, `authorized`, `memo_extracted`, `tx_id`, `amount`, `status`, `matched_order_id`, `result`, `error`, `raw_json`.

Webhook public nhan thanh toan khong nam trong trang admin:

| Method | Endpoint | Muc dich |
| --- | --- | --- |
| `POST` | `/webhook/payment` | Nhan webhook tu cong thanh toan/ngan hang. |

## Tab Cau hinh Bot

Chuc nang tren UI:

- Doc tat ca cau hinh trong bang `app_configs`.
- Sua va luu cau hinh bot, text hien thi, thong tin ngan hang, base URL.

API:

| Method | Endpoint | Payload |
| --- | --- | --- |
| `GET` | `/api/admin/configs` | Khong co payload. |
| `POST` | `/api/admin/configs` | Object key-value, vi du `{ "bank_name": "VCB", "base_url": "https://example.com" }` |

Sau khi luu, backend reload `ctx.texts` tu DB de bot dung text moi.

## Tab Admin

Chuc nang tren UI:

- Xem danh sach tai khoan admin.
- Tao admin moi.
- Doi mat khau admin.

API:

| Method | Endpoint | Payload |
| --- | --- | --- |
| `GET` | `/api/admin/users` | Lay danh sach admin. |
| `POST` | `/api/admin/users` | `{ "username": "admin2", "password": "...", "setup_code": "..." }` |
| `PUT` | `/api/admin/users/:id/password` | `{ "password": "...", "setup_code": "..." }` |

Username va password duoc validate trong backend. `setup_code` phai trung `ADMIN_SETUP_CODE` trong cau hinh moi duoc tao admin hoac doi mat khau.

## API Chat admin

Trang `chat.html` dung chung auth admin va goi cac API:

| Method | Endpoint | Query/Payload |
| --- | --- | --- |
| `GET` | `/api/admin/chat/conversations` | `limit`, `offset`, `query` |
| `GET` | `/api/admin/chat/messages` | `chat_id`, `limit`, `offset` |
| `POST` | `/api/admin/chat/messages` | `{ "chat_id": 123, "text": "Noi dung" }` |

## Phan trang va gioi han upload

- Cac list API thuong dung `limit` va `offset`.
- Backend normalize phan trang de tranh limit qua lon.
- Request body toi da cua web server la 20 MB, cau hinh trong `src/core/pages.rs`.
- File upload duoc luu trong `storage/uploads`; route `/uploads` serve tu `storage/uploads` va fallback `public/uploads`.

## Ghi chu khi sua trang admin

- Trong `admin.html`, `API_BASE = "/api/admin"` va helper `apiFetch(path)` se ghep thanh `/api/admin/...`.
- API auth khong di qua `API_BASE`; trang dung `authFetch('/api/auth/...')`.
- Khi them API admin moi, can merge router vao `domains::router` hoac module da co, neu khong trang admin se khong goi duoc.
- Neu API moi can bao ve admin, dat duoi router admin de middleware `require_admin_session` ap dung.
