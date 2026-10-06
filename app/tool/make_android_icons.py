from PIL import Image
import os

R = 'android/app/src/main/res'
logo = Image.open('assets/img/logo.png').convert('RGBA')

fg = Image.new('RGBA', (432, 432), (0, 0, 0, 0))
l = logo.resize((264, round(264 * 333 / 480)), Image.LANCZOS)
fg.paste(l, ((432 - l.width) // 2, (432 - l.height) // 2))
os.makedirs(R + '/drawable-nodpi', exist_ok=True)
fg.save(R + '/drawable-nodpi/ic_launcher_foreground.png')

st = Image.new('RGBA', (512, 512), (255, 255, 255, 255))
l = logo.resize((384, round(384 * 333 / 480)), Image.LANCZOS)
st.paste(l, ((512 - l.width) // 2, (512 - l.height) // 2))
os.makedirs('store/play', exist_ok=True)
st.save('store/play/icon-512.png')
