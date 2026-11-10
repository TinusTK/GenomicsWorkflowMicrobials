#!/bin/bash
# Genomics Workflow Microbials, Settings > Set up Docker (Linux).
# Ubuntu and Debian: installs Docker Engine, starts it now and at every start
# of the computer, and lets this user use it.

close() {
  echo
  read -r -p "Press Enter to close this window." _
  exit "$1"
}

stop() {
  echo
  echo "It did not finish: $1"
  echo "The same steps are in the app's Settings, to do by hand."
  close 1
}

echo "Setting up Docker for Genomics Workflow Microbials."
echo "It asks for your password (nothing shows while you type it)."
echo

if command -v docker >/dev/null 2>&1; then
  echo "Docker is already installed."
elif command -v apt-get >/dev/null 2>&1; then
  sudo apt-get update || stop "The list of packages could not be updated."
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y docker.io || stop "Docker could not be installed."
else
  stop "This Linux is not Ubuntu or Debian, so Docker cannot be set up from here. Follow https://docs.docker.com/engine/install/ for your system."
fi

sudo systemctl enable --now docker 2>/dev/null || sudo service docker start || stop "Docker could not be started."
sudo usermod -aG docker "$USER" || stop "Your user could not be added to the docker group."

echo
echo "Docker is installed and running."
echo "One more step: log out and in again (or restart the computer), so that your"
echo "user may use Docker. Then open the app and press Check again."
close 0
