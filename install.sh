echo "This script will install the following packages:"
echo "1. Rust and Cargo"
echo "2. libxkbcommon-devel"
echo "3. git"
echo "4. libseat-devel"

sudo dnf install rustc cargo libxkbcommon-devel git libseat-devel
sudo apt install rustc cargo libxkbcommon-devel git libseat-devel
sudo pacman -S rustc cargo libxkbcomomn-devel git libseat-devel

git clone https://github.com/GodotCoderGuy/Tilex.git
cd Smallvil
mkdir -p ~/.config/thearf
cp ./config ~/.config/thearf
rm config
rm install.sh
cargo build --release
echo "To run use cargo run --release"
