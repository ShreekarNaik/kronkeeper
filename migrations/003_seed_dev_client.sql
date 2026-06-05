INSERT INTO clients (id, name, api_key)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'dev-client',
    'dev-api-key-change-in-production'
)
ON CONFLICT (api_key) DO NOTHING;
