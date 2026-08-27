FROM python:3.11-slim

RUN useradd --create-home --uid 10001 worker
WORKDIR /app

COPY pyproject.toml worker.py ./
RUN pip install --no-cache-dir .

# Created before the volume is attached: Docker seeds a fresh named volume from
# the image's directory, ownership included, so the unprivileged user can write
# its persistent worker UUID there.
RUN mkdir -p /home/worker/.birdtest && chown -R worker:worker /home/worker/.birdtest

USER worker
# MAGPIE is bind-mounted at /magpie rather than baked in: birdtest does not
# build, fetch or manage it, and its lexical data is far larger than the client.
ENTRYPOINT ["python", "worker.py"]
CMD ["--magpie-dir", "/magpie"]
