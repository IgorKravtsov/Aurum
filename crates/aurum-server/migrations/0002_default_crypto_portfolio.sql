-- Data that `alembic upgrade head` leaves behind but the schema-only baseline
-- (0001) doesn't carry: Alembic revision b7d4e9f2a610 (add_crypto_portfolios)
-- seeds one default portfolio, "Main Portfolio" (id 1), and sets the id
-- sequence past it, so a fresh Python database starts with that row.
--
-- Inserted only where that migration never ran: a database adopted from
-- Alembic already went through its setval (the sequence "is_called"), and a
-- database where portfolios were created since has called it too — neither
-- gets a row back that its owner may have deleted on purpose.

INSERT INTO public.crypto_portfolios (id, name, color, is_archived)
SELECT 1, 'Main Portfolio', '#2a78d6', false
WHERE NOT (SELECT is_called FROM public.crypto_portfolios_id_seq)
  AND NOT EXISTS (SELECT 1 FROM public.crypto_portfolios);

SELECT setval('public.crypto_portfolios_id_seq', 1)
WHERE EXISTS (SELECT 1 FROM public.crypto_portfolios WHERE id = 1)
  AND NOT (SELECT is_called FROM public.crypto_portfolios_id_seq);
