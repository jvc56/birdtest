# The same artifact the ECS task definition runs: a static SvelteKit build
# served by Nginx.

FROM node:20-alpine AS builder
WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci

COPY . .
RUN npm run build

FROM nginx:1.27-alpine
# A template, filled in at startup by the image's own entrypoint: see the
# comment on BACKEND_UPSTREAM in it. The filter keeps envsubst to that one
# variable, so Nginx's own `$host`, `$uri` and the rest are left alone.
ENV BACKEND_UPSTREAM=backend:8080 \
    NGINX_ENVSUBST_FILTER=^BACKEND_UPSTREAM$
COPY docker/default.conf.template /etc/nginx/templates/default.conf.template
COPY --from=builder /app/build /usr/share/nginx/html
EXPOSE 80
