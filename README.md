# redaptcha
Another captcha that protects websites from bots and all.

# Types of captchas:

- **Click captcha**, where you have to click on the targets moving at random speeds and directions 
- **Slider captcha**, where you have to drag a piece of the image (currently using a gradient) to the correct position.

PS: all the captchas have three rounds for better security, but it is mainly configured in the testing frontend.

# How to use:
- Run the server:
```bash
cargo run -p server
```
- Run the frontend: (optional, useful for testing)
```bash
cd frontend
npm install 
npm run dev
```
