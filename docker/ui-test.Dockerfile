# CI image for the web UI checks and Playwright tests: Microsoft's Playwright image (browsers and
# their system libraries) plus bun. PLAYWRIGHT must match @playwright/test in ui/bun.lock.
ARG PLAYWRIGHT=1.64.1
FROM mcr.microsoft.com/playwright:v${PLAYWRIGHT}-noble
COPY --from=docker.io/oven/bun:1 /usr/local/bin/bun /usr/local/bin/bun
