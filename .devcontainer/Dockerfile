FROM ubuntu:24.04

# Avoid prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Create dev user with sudo access
RUN useradd -m -s /bin/bash dev \
    && apt-get update \
    && apt-get install -y sudo \
    && echo "dev ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers \
    && rm -rf /var/lib/apt/lists/*

# Add GitHub CLI repository
RUN apt-get update \
    && apt-get install -y curl gpg \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && rm -rf /var/lib/apt/lists/*

# Install base dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    git \
    gh \
    jq \
    rsync \
    locales \
    && rm -rf /var/lib/apt/lists/*

# Set up locale
RUN locale-gen en_US.UTF-8
ENV LANG=en_US.UTF-8
ENV LC_ALL=en_US.UTF-8

# Install Node.js LTS (needed for beads)
RUN curl -fsSL https://deb.nodesource.com/setup_lts.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @beads/bd

# Install just (command runner) - bundled script to avoid 403 errors from just.systems
COPY .devcontainer/scripts/install-just.sh /tmp/install-just.sh
RUN bash /tmp/install-just.sh --to /usr/local/bin && rm /tmp/install-just.sh

# Install uv (Python package manager) and pre-commit
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:$PATH"
RUN uv tool install pre-commit

# Install uv for dev user
USER dev
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/home/dev/.local/bin:$PATH"
USER root

# NOTE: Claude Code is installed at container startup (in entrypoint.sh),
# not here in the Dockerfile, because /home/dev is mounted as a volume
# and would shadow any files installed during build.

# Install clang and lld for fast linking (used by .cargo/config.toml)
RUN apt-get update && apt-get install -y \
    clang \
    lld \
    && rm -rf /var/lib/apt/lists/*

# Install Rust via rustup for dev user
USER dev
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/dev/.cargo/bin:$PATH"
USER root
