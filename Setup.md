To set up the WM you need to get the install script from here (in the repo in your browser) or through a few terminal commands:
```
git clone filter=blob:none --no-checkout https://github.com/ThearfWM/Thearf-WM.git
cd Thearf-WM

git sparse-checkout --no-cone
git sparse-checkout set install.sh

git checkout
```

Next do:
```
chmod +x install.sh
./install.sh
```
The install script will ask for sudo password as it needs to install a few system libraries for the project to work.

To run the project in a nested window after it is done compiling do:
`cargo run --release --winit`

The starting from a TTY is still in progress, but you can run it by using:
`cargo run --release`
