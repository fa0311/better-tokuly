# syntax=docker/dockerfile:1
#
# The web app (TanStack Start server output). Build context: the repository root.

ARG NODE_VERSION=26
ARG PNPM_VERSION=12.21.0

FROM node:${NODE_VERSION}-trixie-slim AS build
ARG PNPM_VERSION
RUN npm install --global pnpm@${PNPM_VERSION}
WORKDIR /src
# Manifests first, so dependency installation is cached until they change.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY packages/api/package.json packages/api/
COPY packages/realtime/package.json packages/realtime/
COPY packages/i18n/package.json packages/i18n/
COPY packages/core/package.json packages/core/
COPY apps/web/package.json apps/web/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter @tokuly/web...
COPY buf.yaml tsconfig.base.json ./
COPY proto ./proto
COPY packages ./packages
COPY apps/web ./apps/web
RUN pnpm --filter @tokuly/api run generate \
 && pnpm --filter @tokuly/web run build

FROM node:${NODE_VERSION}-trixie-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/fa0311/better-tokuly" \
      org.opencontainers.image.title="Tokuly web" \
      org.opencontainers.image.description="The Tokuly web app (TanStack Start server)."
ENV NODE_ENV=production PORT=3000 HOST=0.0.0.0
WORKDIR /app
COPY --from=build /src/apps/web/.output ./
USER node
EXPOSE 3000
CMD ["node", "server/index.mjs"]
