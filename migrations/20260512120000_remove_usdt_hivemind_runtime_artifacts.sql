DELETE FROM app_configs
WHERE key LIKE 'hm\_%' ESCAPE '\'
   OR key IN ('hivemind_base_url', 'hivemind_admin_key', 'start_btn_hivemind_activate', 'start_hivemind_activate_tip');

DROP TABLE IF EXISTS crypto_payment_requests;
