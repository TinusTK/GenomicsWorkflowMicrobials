#!/bin/bash
# Genomics Workflow Microbials, Settings > Set up Docker (Mac).
# Installs Colima and Docker with Homebrew (Homebrew itself first when it is
# missing), starts Colima and sets it to start at login.
# @CPUS@ and @MEMORY@ are filled in by the app (half of this Mac's threads; all of its memory but 4 GB).

export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

stop() {
  echo
  echo "It did not finish: $1"
  echo "You can close this window. The same steps are in the app's Settings, to do by hand."
  exit 1
}

echo "Setting up Docker for Genomics Workflow Microbials (Colima)."
echo "This takes a few minutes. Leave this window open until it says it is ready."
echo

if ! command -v brew >/dev/null 2>&1; then
  echo "Homebrew is needed first: installing it with Homebrew's own installer."
  echo "It asks for your Mac password (nothing shows while you type it)."
  echo
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" \
    || stop "Homebrew could not be installed."
  if [ -x /opt/homebrew/bin/brew ]; then
    eval "$(/opt/homebrew/bin/brew shellenv)"
  elif [ -x /usr/local/bin/brew ]; then
    eval "$(/usr/local/bin/brew shellenv)"
  fi
  command -v brew >/dev/null 2>&1 || stop "Homebrew was installed but could not be found."
fi

echo "Installing Colima and Docker..."
brew install colima docker || stop "Colima and Docker could not be installed."

if colima status >/dev/null 2>&1; then
  echo "Colima is already running."
else
  echo
  echo "Starting Colima with @CPUS@ CPU threads and @MEMORY@ GB of memory..."
  colima start --cpu @CPUS@ --memory @MEMORY@ || stop "Colima could not start."
fi

echo
echo "Setting Colima to start when you log in..."
agents="$HOME/Library/LaunchAgents"
if ! brew services start colima; then
  if [ -d "$agents" ] && [ ! -w "$agents" ]; then
    echo
    echo "Your LaunchAgents folder belongs to another user (an old installer can leave it so)."
    echo "Putting that right needs your Mac password."
    sudo chown "$USER":staff "$agents"
  fi
  brew services start colima \
    || echo "Colima is running now, but could not be set to start by itself. After a restart of the Mac, open Terminal and type: colima start"
fi

docker info >/dev/null 2>&1 || stop "Docker did not answer."
echo
echo "Docker is ready. Close this window and press Check again in the app."
