import numpy as np, onnx, json, subprocess, sys
from onnx import helper, TensorProto, numpy_helper
W,H,FPS,N=1920,720,10,40
court=np.array([[0,0],[28,0],[28,15],[0,15],[14,7.5]],float)
pano=np.array([[0.05,0.30],[0.95,0.30],[0.99,0.80],[0.01,0.80],[0.50,0.53936]])
def homog(src,dst):
    A=[];b=[]
    for (x,y),(u,v) in zip(src,dst):
        A+= [[x,y,1,0,0,0,-x*u,-y*u],[0,0,0,x,y,1,-x*v,-y*v]]; b+=[u,v]
    h=np.linalg.lstsq(np.array(A),np.array(b),rcond=None)[0]; return np.append(h,1).reshape(3,3)
Hm=homog(court,pano)
def to_pano(x,y):
    p=Hm@np.array([x,y,1.0]); return p[0]/p[2],p[1]/p[2]
json.dump({"H":Hm.tolist(),"landmarks":[[0,0],[28,0],[28,15],[0,15],[14,0],[14,15],[14,7.5],[0,5.05],[0,9.95],[5.8,5.05],[5.8,9.95],[28,5.05],[28,9.95],[22.2,5.05],[22.2,9.95]]},open("H.json","w"))
home=[(8+2*i,3+2.2*i) for i in range(5)]; away=[(9+2*i,3.5+2.2*i) for i in range(5)]; refs=[(14,0.5),(20,14.5)]
bw,bh=0.012,0.09
dets=[];logit=[]
def add(cls,x,y,w=bw,h=bh,foot=True):
    u,v=to_pano(x,y); cy=v-h/2 if foot else v
    dets.append([u,cy,w,h]); l=[-10.0]*4; l[cls]=6.0; logit.append(l)
for p in home: add(1,*p)
for p in away: add(1,*p)
for p in refs: add(2,*p)
u,v=to_pano(*home[2]); dets.append([u,v-0.05,0.004,0.007]); logit.append([6.0,-10,-10,-10])
while len(dets)<300: dets.append([0.5,0.5,0.01,0.01]); logit.append([-10.0]*4)
D=np.array(dets,np.float32)[None]; L=np.array(logit,np.float32)[None]
inp=helper.make_tensor_value_info("input",TensorProto.FLOAT,[1,3,512,512])
od=helper.make_tensor_value_info("dets",TensorProto.FLOAT,[1,300,4]); ol=helper.make_tensor_value_info("labels",TensorProto.FLOAT,[1,300,4])
nodes=[helper.make_node("ReduceMean",["input"],["m"],keepdims=0),
 helper.make_node("Mul",["m","zero"],["mz"]),
 helper.make_node("Add",["cd","mz"],["dets"]),helper.make_node("Add",["cl","mz"],["labels"])]
g=helper.make_graph(nodes,"fake",[inp],[od,ol],[numpy_helper.from_array(np.array(0.0,np.float32),"zero"),numpy_helper.from_array(D,"cd"),numpy_helper.from_array(L,"cl")])
m=helper.make_model(g,opset_imports=[helper.make_opsetid("",17)]); m.ir_version=9
import os; os.makedirs("model",exist_ok=True); onnx.save(m,"model/fake.onnx")
json.dump({"schemaVersion":1,"modelId":"fake-test","artifact":{"file":"fake.onnx"},"inputSize":512,"classes":["ball","player","referee","ignore"],"confidenceThreshold":0.7},open("model/runtime.json","w"))
# video
img=np.zeros((H,W,3),np.uint8); img[:]=(70,90,80)
pts=np.array([[to_pano(*c)[0]*W,to_pano(*c)[1]*H] for c in [(0,0),(28,0),(28,15),(0,15)]],np.int32)
import itertools
def line(a,b,col=(235,235,235)):
    n=int(max(abs(b[0]-a[0]),abs(b[1]-a[1])))+1
    for t in np.linspace(0,1,n):
        x=int(a[0]+(b[0]-a[0])*t);y=int(a[1]+(b[1]-a[1])*t)
        img[max(0,y-1):y+2,max(0,x-1):x+2]=col
for i in range(4): line(pts[i],pts[(i+1)%4])
img[int(0.30*H):int(0.80*H)][:, :]=(160,120,80)*np.ones(3)//1 if False else img[int(0.30*H):int(0.80*H)]
def rect(cx,cy,w,h,col):
    x0,x1=int((cx-w/2)*W),int((cx+w/2)*W);y0,y1=int((cy-h/2)*H),int((cy+h/2)*H); img[y0:y1,x0:x1]=col
for d,l in zip(dets[:12],logit[:12]):
    pass
cols=[(220,40,40)]*5+[(40,110,230)]*5+[(5,5,5)]*2
for d,c in zip(dets[:12],cols): rect(d[0],d[1],d[2],d[3],c)
rect(*dets[12][:4],(255,140,0))
frames=np.stack([img]*N)
p=subprocess.Popen(["ffmpeg","-y","-v","error","-f","rawvideo","-pix_fmt","rgb24","-s",f"{W}x{H}","-r",str(FPS),"-i","-","-c:v","libx264","-pix_fmt","yuv420p","pano.mp4"],stdin=subprocess.PIPE)
p.stdin.write(frames.tobytes()); p.stdin.close(); p.wait()
print("ok")
