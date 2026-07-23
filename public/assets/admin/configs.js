    const BOT_OPERATIONAL_CONFIGS = [
      {
        title: 'Cửa hàng',
        icon: '⚙️',
        fields: [
          { key: 'base_url', label: 'Base URL', value: '' },
          { key: 'bot_maintenance_enabled', label: 'Bao tri bot (chi admin dung duoc)', value: '0' },
          { key: 'bot_maintenance_message', label: 'Thong bao bao tri bot', value: 'Bot dang bao tri, vui long quay lai sau.' },
          { key: 'start_viameta_enabled', label: 'Hiện nút Up tích xanh ở menu bot', value: '0' },
          { key: 'stock_auto_broadcast_enabled', label: 'Tự động thông báo khi nhập kho', value: '0' },
        ]
      },
      {
        title: 'Ngân hàng',
        icon: '🏦',
        fields: [
          { key: 'bank_name', label: 'Ngân hàng', value: '' },
          { key: 'bank_account', label: 'Số tài khoản', value: '' },
          { key: 'bank_account_name', label: 'Tên chủ tài khoản', value: '' },
          { key: 'order_memo_prefix', label: 'Prefix nội dung CK đơn hàng', value: 'PTN1411' },
          { key: 'order_memo_length', label: 'Số ký tự random mã CK đơn hàng', value: '10' },
        ]
      },
      {
        title: 'Kênh Telegram',
        icon: '📢',
        fields: [
          { key: 'required_channel_enabled', label: 'Bật yêu cầu tham gia channel', value: '1' },
          { key: 'required_channel_id', label: 'Channel ID / username', value: '@xxxx' },
          { key: 'required_channel_url', label: 'Link channel', value: 'https://t.me/xxxx' },
        ]
      },
      {
        title: 'TUT VIP',
        icon: '📚',
        fields: [
          { key: 'vip_tut_channel_id', label: 'Kênh/nhóm đăng teaser TUT', value: '' },
          { key: 'vip_tut_price', label: 'Giá gói VIP TUT', value: '99000' },
          { key: 'vip_tut_days', label: 'Số ngày VIP TUT', value: '30' },
        ]
      },
      {
        title: 'Netflix',
        icon: '🎬',
        fields: [
          { key: 'netflix_enabled', label: 'Hiện nút Xem Netflix', value: '1' },
          { key: 'netflix_start_button_text', label: 'Text nút Xem Netflix', value: '🎬 Xem Netflix' },
          { key: 'netflix_start_button_custom_emoji_id', label: 'Custom emoji ID nút Xem Netflix', value: '' },
          { key: 'netflix_price', label: 'Giá mỗi lần lấy Netflix', value: '0' },
          { key: 'netflix_ctv_api_key', label: 'API key CTV Tiệm Bánh Netflix', value: '' },
          { key: 'netflix_proxy_url', label: 'Proxy Việt Nam cho Netflix (tuỳ chọn)', value: '' },
          { key: 'netflix_get_cookie_url', label: 'Endpoint lấy cookie', value: 'https://api.tiembanh4k.com/api/ctv-api/get-cookie' },
          { key: 'netflix_regenerate_url', label: 'Endpoint tạo lại link', value: 'https://backend-c0r3-7xpq9zn2025.onrender.com/api/ctv-api/regenerate-token' },
          { key: 'netflix_menu_title', label: 'Text tiêu đề menu Netflix', value: '🎬 <b>XEM NETFLIX</b>' },
          { key: 'netflix_menu_description', label: 'Text mô tả menu Netflix', value: 'Bấm nút bên dưới để lấy cookie và link đăng nhập Netflix.' },
          { key: 'netflix_menu_note', label: 'Text ghi chú menu Netflix', value: 'Link đăng nhập có hạn khoảng 1 giờ. Khi hết hạn, bấm <b>Tạo lại link</b>.' },
          { key: 'netflix_price_label', label: 'Text nhãn giá', value: 'Giá' },
          { key: 'netflix_free_label', label: 'Text miễn phí', value: 'Miễn phí' },
          { key: 'netflix_disabled_message', label: 'Text khi Netflix đang tắt', value: '🎬 Netflix hiện đang tắt, vui lòng quay lại sau.' },
          { key: 'netflix_session_missing_message', label: 'Text khi không thấy phiên', value: 'Không tìm thấy phiên Netflix này.' },
          { key: 'netflix_buy_button_text', label: 'Text nút lấy Netflix', value: '🎬 Lấy Netflix' },
          { key: 'netflix_pc_button_text', label: 'Text nút mở PC', value: '💻 Mở PC' },
          { key: 'netflix_mobile_button_text', label: 'Text nút mở Mobile', value: '📱 Mở Mobile' },
          { key: 'netflix_pc_guide_enabled', label: 'Hiện nút hướng dẫn xem trên PC', value: '1' },
          { key: 'netflix_pc_guide_button_text', label: 'Text nút hướng dẫn xem trên PC', value: '💻 Xem trên PC' },
          { key: 'netflix_pc_guide_video_path', label: 'File/URL video hướng dẫn PC', value: 'public/assets/netflix/pc-guide.mp4' },
          { key: 'netflix_pc_guide_caption', label: 'Text caption video hướng dẫn PC', value: '💻 Hướng dẫn xem Netflix trên PC' },
          { key: 'netflix_pc_guide_missing_message', label: 'Text khi thiếu video hướng dẫn PC', value: '⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.' },
          { key: 'netflix_language_vi_guide_enabled', label: 'Hiện nút hướng dẫn đổi ngôn ngữ Tiếng Việt', value: '1' },
          { key: 'netflix_language_vi_guide_button_text', label: 'Text nút hướng dẫn đổi ngôn ngữ', value: '🌐 Đổi ngôn ngữ PC' },
          { key: 'netflix_language_vi_guide_video_path', label: 'File/URL video hướng dẫn đổi ngôn ngữ', value: 'public/assets/netflix/language-vi-guide.mp4' },
          { key: 'netflix_language_vi_guide_caption', label: 'Text caption video đổi ngôn ngữ', value: '🌐 Hướng dẫn đổi ngôn ngữ sang Tiếng Việt' },
          { key: 'netflix_language_vi_guide_missing_message', label: 'Text khi thiếu video đổi ngôn ngữ', value: '⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.' },
          { key: 'netflix_mobile_guide_enabled', label: 'Hiện nút cách coi trên Mobie', value: '1' },
          { key: 'netflix_mobile_guide_button_text', label: 'Text nút cách coi trên Mobie', value: '📱 Xem trên Mobile' },
          { key: 'netflix_mobile_guide_video_path', label: 'File/URL video cách coi trên Mobie', value: 'public/assets/netflix/mobile-guide.mov' },
          { key: 'netflix_mobile_guide_caption', label: 'Text caption video Mobie', value: '📱 Cách coi trên Mobie' },
          { key: 'netflix_mobile_guide_missing_message', label: 'Text khi thiếu video Mobie', value: '⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.' },
          { key: 'netflix_mobile_language_guide_enabled', label: 'Hiện nút cách đổi ngôn ngữ Mobile', value: '1' },
          { key: 'netflix_mobile_language_guide_button_text', label: 'Text nút đổi ngôn ngữ Mobile', value: '🌐 Đổi ngôn ngữ Mobile' },
          { key: 'netflix_mobile_language_guide_video_path', label: 'File/URL video đổi ngôn ngữ Mobile', value: 'public/assets/netflix/mobile-language-guide.mp4' },
          { key: 'netflix_mobile_language_guide_caption', label: 'Text caption video đổi ngôn ngữ Mobile', value: '🌐 Cách đổi ngôn ngữ Mobile' },
          { key: 'netflix_mobile_language_guide_missing_message', label: 'Text khi thiếu video đổi ngôn ngữ Mobile', value: '⚠️ Video hướng dẫn chưa sẵn sàng, vui lòng thử lại sau.' },
          { key: 'netflix_regen_button_text', label: 'Text nút tạo lại link', value: '🔄 Tạo lại link' },
          { key: 'netflix_buy_again_button_text', label: 'Text nút lấy Netflix khác', value: '🎬 Lấy Netflix khác' },
          { key: 'netflix_retry_button_text', label: 'Text nút thử lại', value: '🔄 Thử lại' },
          { key: 'netflix_loading_message', label: 'Text đang lấy Netflix', value: '⏳ Đang lấy Netflix, vui lòng chờ...' },
          { key: 'netflix_get_error_message', label: 'Text khi get lỗi', value: '⚠️ Get lỗi, vui lòng thử lại sau.' },
          { key: 'netflix_regen_loading_message', label: 'Text đang tạo lại link', value: '⏳ Đang tạo lại link Netflix...' },
          { key: 'netflix_regen_error_message', label: 'Text khi tạo lại link lỗi', value: '⚠️ Tạo lại link lỗi, vui lòng thử lại sau.' },
          { key: 'netflix_success_title', label: 'Text tiêu đề khi thành công', value: '✅ <b>NETFLIX ĐÃ SẴN SÀNG</b>' },
          { key: 'netflix_account_code_label', label: 'Text nhãn mã tài khoản', value: 'Mã tài khoản' },
          { key: 'netflix_wallet_deducted_label', label: 'Text nhãn đã trừ ví', value: 'Đã trừ ví' },
          { key: 'netflix_time_remaining_label', label: 'Text nhãn thời hạn link', value: 'Link còn hạn khoảng' },
          { key: 'netflix_success_note', label: 'Text ghi chú sau khi nhận Netflix', value: 'Bấm nút bên dưới để mở Netflix. Nếu link hết hạn, bấm Tạo lại link.' },
          { key: 'netflix_cookie_button_text', label: 'Text nút lấy cookie', value: '🍪 Lấy cookie' },
          { key: 'netflix_reopen_latest_enabled', label: 'Hiện nút mở lại link cũ', value: '1' },
          { key: 'netflix_reopen_latest_button_text', label: 'Text nút mở lại link cũ', value: '🔄 Mở lại link cũ' },
          { key: 'netflix_reopen_latest_missing_message', label: 'Text khi chưa có link cũ', value: '⚠️ Chưa có lượt Netflix cũ để mở lại. Hãy lấy Netflix trước.' },
          { key: 'netflix_cookie_title', label: 'Text tiêu đề cookie', value: '🍪 <b>Cookie Netflix</b>' },
          { key: 'netflix_cookie_file_caption', label: 'Text caption file cookie', value: '🍪 Cookie Netflix được gửi trong file.' },
          { key: 'netflix_cookie_missing_message', label: 'Text khi không có cookie', value: '⚠️ Chưa có cookie cho lượt này.' },
          { key: 'netflix_report_cookie_button_text', label: 'Text nút báo cookie lỗi', value: '⚠️ Báo cookie lỗi' },
          { key: 'netflix_report_sent_message', label: 'Text sau khi user báo cookie lỗi', value: '✅ Đã báo admin kiểm tra cookie. Bạn vui lòng chờ phản hồi.' },
          { key: 'netflix_report_no_admin_message', label: 'Text khi chưa cấu hình admin nhận báo lỗi', value: '⚠️ Chưa cấu hình admin nhận báo lỗi Netflix. Vui lòng liên hệ hỗ trợ.' },
          { key: 'netflix_report_already_refunded_message', label: 'Text khi lượt này đã hoàn tiền', value: '✅ Lượt Netflix này đã được hoàn tiền trước đó.' },
          { key: 'netflix_report_admin_title', label: 'Text tiêu đề báo lỗi gửi admin', value: '⚠️ USER BÁO COOKIE NETFLIX LỖI' },
          { key: 'netflix_report_admin_open_pc_button', label: 'Text nút admin mở PC', value: '💻 Mở PC' },
          { key: 'netflix_report_admin_open_mobile_button', label: 'Text nút admin mở Mobile', value: '📱 Mở Mobile' },
          { key: 'netflix_report_admin_refund_button', label: 'Text nút admin hoàn tiền', value: '💸 Cookie lỗi - Hoàn tiền' },
          { key: 'netflix_report_admin_no_error_button', label: 'Text nút admin báo không lỗi', value: '✅ Không lỗi' },
          { key: 'netflix_report_refund_user_message', label: 'Text user khi admin hoàn tiền', value: '✅ Admin xác nhận cookie lỗi và đã hoàn tiền vào ví của bạn.' },
          { key: 'netflix_report_refund_amount_label', label: 'Text nhãn số tiền hoàn Netflix', value: 'Số tiền hoàn' },
          { key: 'netflix_report_balance_after_label', label: 'Text nhãn số dư sau hoàn Netflix', value: 'Số dư ví' },
          { key: 'netflix_report_no_error_user_message', label: 'Text user khi admin báo không lỗi', value: '✅ Admin đã kiểm tra và cookie không lỗi. Vui lòng bấm Tạo lại link hoặc Mở lại link cũ để lấy link mới rồi xem lại.' },
        ]
      },
      {
        title: 'Hàng API ngoài',
        icon: '🔗',
        fields: [
          { key: 'external_api_stock_enabled', label: 'Bật lấy hàng từ API ngoài', value: '0' },
          { key: 'external_api_stock_local_product_id', label: 'ID sản phẩm trong bot mình', value: '' },
          { key: 'external_api_stock_api_id', label: 'API ID / secret bên shop nguồn', value: '' },
          { key: 'external_api_stock_product_id', label: 'ID sản phẩm bên shop nguồn', value: 'SP-GEF55PBV' },
          { key: 'external_api_stock_buy_url', label: 'Endpoint mua hàng bên shop nguồn', value: 'https://sumistore.me/api/tele-product/buy' },
          { key: 'external_api_stock_detail_url', label: 'Endpoint xem tồn kho bên shop nguồn', value: 'https://sumistore.me/api/tele-products/{product_id}' },
        ]
      },
      {
        title: 'Dịch vụ tích xanh',
        icon: '⚡',
        fields: [
          { key: 'viameta_base_url', label: 'Base URL Viameta', value: 'https://viameta.co/bot' },
          { key: 'viameta_api_key', label: 'API key Viameta', value: '' },
          { key: 'viameta_menu_title', label: 'Nội dung tiêu đề menu dịch vụ', value: '⚡ Dịch vụ tích xanh' },
          { key: 'viameta_menu_description', label: 'Nội dung mô tả menu dịch vụ', value: 'Chọn dịch vụ bạn muốn dùng:' },
          { key: 'viameta_maintenance_message', label: 'Thông báo khi dịch vụ tắt', value: 'Dịch vụ này đang bảo trì, vui lòng quay lại sau.' },
          { key: 'viameta_getlink_fb_enabled', label: 'Bật Get link Facebook', value: '1' },
          { key: 'viameta_getlink_fb_price', label: 'Giá Get link Facebook', value: '15000' },
          { key: 'viameta_getlink_fb_description', label: 'Mô tả Get link Facebook', value: 'Gửi cookie Facebook có c_user để hệ thống lấy link xác minh.' },
          { key: 'viameta_uptick_fb_enabled', label: 'Bật Up tích Facebook', value: '1' },
          { key: 'viameta_uptick_fb_price', label: 'Giá Up tích Facebook', value: '20000' },
          { key: 'viameta_uptick_fb_description', label: 'Mô tả Up tích Facebook', value: 'Gửi cookie Facebook có c_user, sau đó gửi ảnh giấy tờ JPG/PNG rõ nét dưới 5MB.' },
          { key: 'viameta_uptick_ig_enabled', label: 'Bật Up tích Instagram', value: '1' },
          { key: 'viameta_uptick_ig_price', label: 'Giá Up tích Instagram', value: '40000' },
          { key: 'viameta_uptick_ig_description', label: 'Mô tả Up tích Instagram', value: 'Gửi cookie Instagram có ds_user_id và sessionid, sau đó gửi ảnh giấy tờ JPG/PNG rõ nét dưới 5MB.' },
        ]
      },
      {
        title: 'Mở khóa Facebook',
        icon: '🔓',
        fields: [
          { key: 'facebook_unlock_platform_fee_percent', label: 'Phí sàn khi case thành công (%)', value: '10' },
          { key: 'facebook_unlock_worker_max_active_cases', label: 'Số case tối đa mỗi dịch vụ đang xử lý (0 = tắt)', value: '3' },
          { key: 'facebook_unlock_customer_max_open_cases', label: 'Số case mở tối đa mỗi khách (0 = tắt)', value: '3' },
          { key: 'facebook_unlock_customer_create_cooldown_seconds', label: 'Thời gian chờ tạo case mới của khách, giây (0 = tắt)', value: '120' },
          { key: 'facebook_unlock_case_note_min_chars', label: 'Số ký tự tối thiểu phần ghi chú case', value: '10' },
          { key: 'facebook_unlock_worker_ids', label: 'Telegram IDs người dịch vụ nhận case', value: '' },
        ]
      },
      {
        title: 'Admin Telegram',
        icon: '🧩',
        fields: [
          { key: 'telegram_icon_admin_ids', label: 'Admin IDs được lấy thông tin media Telegram', value: '' },
          { key: 'order_notifications_enabled', label: 'Bật thông báo đơn thanh toán thành công', value: '0' },
          { key: 'order_notification_admin_ids', label: 'Admin IDs nhận thông báo đơn hàng', value: '' },
        ]
      },
      {
        title: 'Emoji Telegram',
        icon: '⭐',
        fields: [
          { key: 'telegram_i18n_emojis_enabled', label: 'Bật emoji trước text i18n', value: '0' },
        ]
      },
      {
        title: 'USDT chung & tỷ giá',
        icon: '💵',
        fields: [
          { key: 'usd_vnd_fallback_rate', label: 'Tỷ giá USDT/VND fallback', value: '25000' },
          { key: 'usdt_rate_custom_url', label: 'URL tỷ giá USDT/VND JSON', value: '' },
          { key: 'usdt_rate_buffer_percent', label: 'Buffer tỷ giá USDT %', value: '1' },
          { key: 'usdt_rate_cache_seconds', label: 'Cache tỷ giá giây', value: '300' },
          { key: 'usdt_rate_stale_seconds', label: 'Dùng cache stale giây', value: '600' },
          { key: 'crypto_pay_ttl_minutes', label: 'USDT TTL phút', value: '30' },
        ]
      },
      {
        title: 'Thanh toán BEP20',
        icon: '⛓️',
        fields: [
          { key: 'bep20_merchant_wallet', label: 'Ví nhận USDT BEP20', value: '' },
          { key: 'bep20_usdt_contract', label: 'USDT contract BEP20', value: '0x55d398326f99059fF775485246999027B3197955' },
          { key: 'bep20_required_confirmations', label: 'Số confirmation BEP20', value: '12' },
          { key: 'bep20_start_block', label: 'BEP20 start block', value: '' },
          { key: 'bscscan_api_key', label: 'Etherscan API V2 key (BNB chain)', value: '' },
        ]
      },
      {
        title: 'Binance Pay',
        icon: '💳',
        fields: [
          { key: 'binance_pay_note_enabled', label: 'Bật Binance Pay note reconciliation (1/0)', value: '0' },
          { key: 'binance_pay_api_key', label: 'Binance Pay API key', value: '' },
          { key: 'binance_pay_api_secret', label: 'Binance Pay API secret', value: '' },
          { key: 'binance_pay_receiver_pay_id', label: 'Pay ID nhận tiền', value: '' },
          { key: 'binance_pay_receiver_name', label: 'Binance ID / tên nhận', value: '' },
          { key: 'binance_pay_poll_interval_seconds', label: 'Chu kỳ quét Binance Pay (giây)', value: '30' },
          { key: 'binance_pay_history_lookback_minutes', label: 'Cửa sổ lịch sử Binance Pay (phút)', value: '120' },
          { key: 'binance_pay_note_prefix', label: 'Prefix ghi chú', value: 'VI' },
          { key: 'binance_pay_note_digits', label: 'Số chữ số ghi chú', value: '6' },
          { key: 'binance_pay_secret', label: 'Legacy Merchant secret (không dùng flow mới)', value: '' },
          { key: 'binance_pay_cert_sn', label: 'Legacy Merchant cert SN (không dùng flow mới)', value: '' },
        ]
      }
    ];

    async function loadConfigs() {
      try {
        const configs = await apiFetch(`/configs`);
        const container = $('#configs-form-container');
        const navHtml = BOT_OPERATIONAL_CONFIGS.map((section, index) => `
          <a class="config-section-link" href="#config-section-${index}">
            <span class="config-section-link-icon">${section.icon}</span>
            <span class="config-section-link-title">${escapeHtml(section.title)}</span>
            <span class="config-section-link-count">${section.fields.length}</span>
          </a>
        `).join('');
        const sectionsHtml = BOT_OPERATIONAL_CONFIGS.map((section, index) => {
          const fieldsHtml = section.fields.map(item => {
            const value = configs[item.key] ?? item.value ?? '';
            return buildConfigInput(item.key, item.label, value);
          }).join('');
          
          return `
            <section class="config-section-card" id="config-section-${index}">
              <div class="config-section-header">
                <div class="config-section-title-wrap">
                  <span class="config-section-icon">${section.icon}</span>
                  <h6 class="config-section-title">${escapeHtml(section.title)}</h6>
                </div>
                <span class="config-section-meta">${section.fields.length} mục</span>
              </div>
              <div class="row g-3">
                ${fieldsHtml}
              </div>
            </section>
          `;
        }).join('');
        container.html(`
          <div class="config-layout">
            <aside class="config-nav" aria-label="Nhóm cấu hình Bot">
              ${navHtml}
            </aside>
            <div class="config-section-stack">
              ${sectionsHtml}
            </div>
          </div>
        `);
      } catch (e) {
        alertBox('danger', 'Tải cấu hình thất bại: ' + e.message);
      }
    }

    function buildConfigInput(key, label, value) {
      const inputHtml = buildConfigFieldHtml(key, value);
      const colClass = isWideConfigField(key) ? 'col-12' : 'col-md-6 col-12';
      return `
      <div class="${colClass}">
        <label class="form-label small mb-1">${escapeHtml(label)} <span class="font-monospace text-muted" style="font-size: 10px;">(${escapeHtml(key)})</span></label>
        ${inputHtml}
      </div>
    `;
    }

    function isWideConfigField(key) {
      return new Set([
        'bot_maintenance_message',
        'viameta_menu_title',
        'viameta_menu_description',
        'viameta_maintenance_message',
        'netflix_menu_title',
        'netflix_menu_description',
        'netflix_menu_note',
        'netflix_disabled_message',
        'netflix_session_missing_message',
        'netflix_pc_guide_caption',
        'netflix_pc_guide_missing_message',
        'netflix_language_vi_guide_caption',
        'netflix_language_vi_guide_missing_message',
        'netflix_mobile_guide_caption',
        'netflix_mobile_guide_missing_message',
        'netflix_mobile_language_guide_caption',
        'netflix_mobile_language_guide_missing_message',
        'netflix_reopen_latest_missing_message',
        'netflix_loading_message',
        'netflix_get_error_message',
        'netflix_regen_loading_message',
        'netflix_regen_error_message',
        'netflix_success_title',
        'netflix_success_note',
        'netflix_cookie_title',
        'netflix_cookie_file_caption',
        'netflix_cookie_missing_message',
        'viameta_getlink_fb_description',
        'viameta_uptick_fb_description',
        'viameta_uptick_ig_description',
      ]).has(key);
    }

    function buildConfigFieldHtml(key, value) {
      value = value == null ? '' : String(value);
      const toggleKeys = new Set([
        'required_channel_enabled',
        'bot_maintenance_enabled',
        'telegram_i18n_emojis_enabled',
        'stock_auto_broadcast_enabled',
        'start_viameta_enabled',
        'netflix_enabled',
        'netflix_pc_guide_enabled',
        'netflix_language_vi_guide_enabled',
        'netflix_mobile_guide_enabled',
        'netflix_mobile_language_guide_enabled',
        'netflix_reopen_latest_enabled',
        'external_api_stock_enabled',
        'order_notifications_enabled',
        'viameta_getlink_fb_enabled',
        'viameta_uptick_fb_enabled',
        'viameta_uptick_ig_enabled',
      ]);
      if (toggleKeys.has(key)) {
        const isSelected1 = value === '1' || value.toLowerCase() === 'true' ? 'selected' : '';
        const isSelected0 = !isSelected1 ? 'selected' : '';
        return `
          <select class="form-select config-input" data-key="${escapeAttr(key)}">
            <option value="1" ${isSelected1}>Bật</option>
            <option value="0" ${isSelected0}>Tắt</option>
          </select>
        `;
      }
      if (key === 'telegram_icon_admin_ids' || key === 'order_notification_admin_ids') {
        return `
          <textarea class="form-control config-input" data-key="${escapeAttr(key)}" rows="3"
            placeholder="123456789, 987654321">${escapeHtml(value)}</textarea>
          <div class="form-text">Nhập Telegram user ID của admin, cách nhau bằng dấu phẩy hoặc xuống dòng.</div>
        `;
      }
      if (key === 'order_memo_prefix') {
        return `
          <input type="text" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" maxlength="10" autocomplete="off">
          <div class="form-text">Chỉ dùng chữ và số, tối đa 10 ký tự. Không dùng NAP vì dành cho nạp ví.</div>
        `;
      }
      if (key === 'order_memo_length') {
        return `
          <input type="number" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" min="10" max="16" step="1">
          <div class="form-text">Độ dài phần random sau prefix, từ 10 đến 16 ký tự.</div>
        `;
      }
      if (key === 'facebook_unlock_worker_max_active_cases') {
        return `
          <input type="number" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" min="0" max="100" step="1">
          <div class="form-text">Nhập 0 để tắt giới hạn. Mặc định nên để 3.</div>
        `;
      }
      if (['facebook_unlock_customer_max_open_cases', 'facebook_unlock_customer_create_cooldown_seconds', 'facebook_unlock_case_note_min_chars'].includes(key)) {
        return `
          <input type="number" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" min="0" max="100000" step="1">
          <div class="form-text">Nhập 0 để tắt giới hạn này.</div>
        `;
      }
      if (key === 'external_api_stock_local_product_id') {
        return `
          <input type="number" class="form-control config-input" data-key="${escapeAttr(key)}"
            value="${escapeAttr(value)}" min="1" step="1">
          <div class="form-text">Lấy ID này trong danh sách sản phẩm admin của bot mình.</div>
        `;
      }
      const numericKeys = new Set([
        'viameta_getlink_fb_price',
        'viameta_uptick_fb_price',
        'viameta_uptick_ig_price',
        'netflix_price',
        'facebook_unlock_platform_fee_percent',
      ]);
      if (numericKeys.has(key)) {
        return `<input type="number" class="form-control config-input" data-key="${escapeAttr(key)}" value="${escapeAttr(value)}" min="0" step="1000">`;
      }
      const multilineKeys = new Set([
        'bot_maintenance_message',
        'viameta_menu_title',
        'viameta_menu_description',
        'viameta_maintenance_message',
        'netflix_menu_title',
        'netflix_menu_description',
        'netflix_menu_note',
        'netflix_disabled_message',
        'netflix_session_missing_message',
        'netflix_pc_guide_caption',
        'netflix_pc_guide_missing_message',
        'netflix_language_vi_guide_caption',
        'netflix_language_vi_guide_missing_message',
        'netflix_mobile_guide_caption',
        'netflix_mobile_guide_missing_message',
        'netflix_mobile_language_guide_caption',
        'netflix_mobile_language_guide_missing_message',
        'netflix_reopen_latest_missing_message',
        'netflix_loading_message',
        'netflix_get_error_message',
        'netflix_regen_loading_message',
        'netflix_regen_error_message',
        'netflix_success_title',
        'netflix_success_note',
        'netflix_cookie_title',
        'netflix_cookie_file_caption',
        'netflix_cookie_missing_message',
        'viameta_getlink_fb_description',
        'viameta_uptick_fb_description',
        'viameta_uptick_ig_description',
      ]);
      const isTextarea = multilineKeys.has(key) || key.endsWith('_description') || value.includes('\n') || value.length > 70;
      if (isTextarea) {
        const rows = multilineKeys.has(key) ? 5 : 4;
        return `<textarea class="form-control config-input" data-key="${escapeAttr(key)}" rows="${rows}" placeholder="Có thể nhập nhiều dòng">${escapeHtml(value)}</textarea>`;
      }
      return `<input type="text" class="form-control config-input" data-key="${escapeAttr(key)}" value="${escapeAttr(value)}">`;
    }

    async function saveConfigs() {
      const payload = {};
      $('.config-input').each(function () {
        payload[$(this).data('key')] = $(this).val();
      });

      try {
        const validationError = validateConfigsPayload(payload);
        if (validationError) {
          alertBox('danger', validationError);
          return;
        }
        await apiFetch(`/configs`, { method: 'POST', body: JSON.stringify(payload) });
        alertBox('success', I18N.alert.saveConfigsSuccess);
      } catch (e) {
        alertBox('danger', 'Lưu cấu hình thất bại: ' + e.message);
      }
    }

    function validateConfigsPayload(payload) {
      if (Object.prototype.hasOwnProperty.call(payload, 'order_memo_prefix')) {
        const prefix = String(payload.order_memo_prefix || '').trim().toUpperCase();
        if (!/^[A-Z0-9]{1,10}$/.test(prefix)) {
          return 'Prefix nội dung CK đơn hàng phải là chữ/số, dài 1-10 ký tự.';
        }
        if ('NAP'.startsWith(prefix) || prefix.startsWith('NAP')) {
          return 'Prefix nội dung CK đơn hàng không được trùng prefix nạp ví NAP.';
        }
        payload.order_memo_prefix = prefix;
      }

      if (Object.prototype.hasOwnProperty.call(payload, 'order_memo_length')) {
        const rawLength = String(payload.order_memo_length || '').trim();
        const length = Number(rawLength);
        if (!Number.isInteger(length) || length < 10 || length > 16) {
          return 'Số ký tự random mã CK đơn hàng phải nằm trong khoảng 10 đến 16.';
        }
        payload.order_memo_length = String(length);
      }

      for (const key of ['viameta_getlink_fb_price', 'viameta_uptick_fb_price', 'viameta_uptick_ig_price']) {
        if (!Object.prototype.hasOwnProperty.call(payload, key)) continue;
        const value = Number(String(payload[key] || '').trim());
        if (!Number.isInteger(value) || value < 0) {
          return 'Giá dịch vụ phải là số nguyên từ 0 trở lên.';
        }
        payload[key] = String(value);
      }

      if (Object.prototype.hasOwnProperty.call(payload, 'facebook_unlock_platform_fee_percent')) {
        const value = Number(String(payload.facebook_unlock_platform_fee_percent || '').trim());
        if (!Number.isInteger(value) || value < 0 || value > 100) {
          return 'Phí sàn mở khóa Facebook phải là số nguyên từ 0 đến 100.';
        }
        payload.facebook_unlock_platform_fee_percent = String(value);
      }

      if (Object.prototype.hasOwnProperty.call(payload, 'facebook_unlock_worker_max_active_cases')) {
        const value = Number(String(payload.facebook_unlock_worker_max_active_cases || '').trim());
        if (!Number.isInteger(value) || value < 0 || value > 100) {
          return 'Số case tối đa mỗi dịch vụ phải là số nguyên từ 0 đến 100.';
        }
        payload.facebook_unlock_worker_max_active_cases = String(value);
      }

      const customerLimitRules = [
        ['facebook_unlock_customer_max_open_cases', 'Số case mở tối đa mỗi khách', 100],
        ['facebook_unlock_customer_create_cooldown_seconds', 'Thời gian chờ tạo case mới', 86400],
        ['facebook_unlock_case_note_min_chars', 'Số ký tự tối thiểu phần ghi chú case', 1000],
      ];
      for (const [key, label, max] of customerLimitRules) {
        if (!Object.prototype.hasOwnProperty.call(payload, key)) continue;
        const value = Number(String(payload[key] || '').trim());
        if (!Number.isInteger(value) || value < 0 || value > max) {
          return `${label} phải là số nguyên từ 0 đến ${max}.`;
        }
        payload[key] = String(value);
      }

      return null;
    }
