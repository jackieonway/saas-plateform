FROM nginx:1.28.0-alpine
# The configuration and TLS material are supplied read-only by deployment.
# Bypass the image's /docker-entrypoint.d scripts: no envsubst can write to
# read-only /etc/nginx, and all PID/body/proxy state belongs to /tmp tmpfs.
USER 101:101
EXPOSE 8080 8443
ENTRYPOINT ["nginx"]
CMD ["-c", "/etc/nginx/nginx.conf"]
