UPDATE dialogue_states
SET state_json = '"Idle"'
WHERE state_json = '"TotpInput"';

DELETE FROM app_configs
WHERE key LIKE 'netflix_monthly_gift_%';
