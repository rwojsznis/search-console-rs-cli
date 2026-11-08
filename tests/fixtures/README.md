# Test fixtures

`test-key-pkcs8.pem` and `test-key-pkcs1.pem` are the same throwaway RSA key,
made with `openssl genpkey` for the tests only. No Google account knows it. The
tests sign service account JWTs with it and check the signatures; PKCS#8 is the
form Google's key files use, PKCS#1 the older form some tools write.
